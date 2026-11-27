//! The overall request limits: how many RPCs one account, and one client
//! address without an account, may make per minute.
//!
//! Each RPC that can be abused has a limit of its own, written next to it.
//! These come first, in [`crate::auth::auth_middleware`], and catch what a
//! handler forgot: a signed-in account is counted over all of its sessions
//! and devices, a caller without one (no token, or a token that is no longer
//! valid) by its address. IPv6 addresses count per /64, the block one
//! subscriber is given, so walking through the addresses of one network
//! does not reset the count, and the /64s of one /48 share a count of their
//! own: holding a /48 does not give 65536 counts. API tokens have their own
//! limit ([`crate::api_tokens`]) and are not counted here.
//!
//! Only RPCs are counted (`/tokuly.<domain>.v1.<Service>/<Method>`); the few
//! plain HTTP routes behind the middleware (the OAuth endpoints, pictures of
//! link previews) are limited by their domains.
//!
//! A refused call ends with `resource_exhausted` / `RATE_LIMITED` and the
//! `retry-after` and `x-ratelimit-*` headers, the answer clients already
//! know from every other limit. A Valkey that does not answer lets requests
//! through: a guard of the whole API must not take the whole API down.
use std::{net::IpAddr, time::Duration};

use chrono::Utc;
use uuid::Uuid;

use crate::{
    auth::AuthState,
    cache::Cache,
    error::{AppError, AppResult},
    proxies,
};

/// Calls per minute; 0 means no limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestLimits {
    /// For a signed-in account, over all of its sessions.
    pub per_account: u32,
    /// For callers without an account, per address (IPv6: per /64).
    pub per_address: u32,
    /// For callers without an account from IPv6, per /48 over all of its /64s.
    pub per_network: u32,
}

impl RequestLimits {
    /// No limit at all.
    pub const NONE: Self = Self { per_account: 0, per_address: 0, per_network: 0 };
}

/// How long the counter may take before the request goes ahead uncounted.
const COUNT_TIMEOUT: Duration = Duration::from_millis(50);

/// Where a caller stands against a limit in the current minute.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RateStatus {
    pub(crate) limit: u32,
    pub(crate) remaining: u32,
    /// Seconds until the window ends.
    pub(crate) reset_seconds: u32,
    pub(crate) exceeded: bool,
}

impl RateStatus {
    pub(crate) fn write_headers(&self, headers: &mut http::HeaderMap) {
        headers.insert("x-ratelimit-limit", self.limit.into());
        headers.insert("x-ratelimit-remaining", self.remaining.into());
        headers.insert("x-ratelimit-reset", self.reset_seconds.into());
        if self.exceeded {
            headers.insert(http::header::RETRY_AFTER, self.reset_seconds.into());
        }
    }
}

/// Counts one request against `limit` per minute for `key` (fixed one-minute windows).
pub(crate) async fn count_per_minute(cache: &Cache, key: &str, limit: u32) -> AppResult<RateStatus> {
    let now = Utc::now().timestamp();
    let key = format!("rl:{key}:{}", now / 60);
    let (count,): (i64,) =
        redis::pipe().atomic().incr(&key, 1).expire(&key, 120).ignore().query_async(&mut cache.conn()).await?;
    Ok(RateStatus {
        limit,
        remaining: (limit as i64 - count).clamp(0, limit as i64) as u32,
        reset_seconds: (60 - now.rem_euclid(60)) as u32,
        exceeded: count > limit as i64,
    })
}

/// Who a request is counted against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Caller {
    Account(Uuid),
    /// The address, IPv6 cut to its /64.
    Address(IpAddr),
}

impl Caller {
    /// `None` for API tokens (they have their own limit) and for a caller
    /// whose address is unknown.
    fn of(state: &AuthState, address: Option<IpAddr>) -> Option<Self> {
        match state {
            AuthState::Authenticated(ctx) if ctx.is_session() => Some(Self::Account(ctx.user_id)),
            AuthState::Authenticated(_) => None,
            AuthState::Anonymous | AuthState::Invalid(_) => address.map(|address| Self::Address(network_of(address))),
        }
    }

    /// What a request of this caller is counted against: its account, or its
    /// address and, for IPv6, also the /48 around it.
    fn counts(self) -> impl Iterator<Item = Count> {
        let (count, network) = match self {
            Self::Account(user_id) => (Count::Account(user_id), None),
            Self::Address(address) => {
                (Count::Address(address), address.is_ipv6().then(|| Count::Network(proxies::masked(address, 48))))
            }
        };
        std::iter::once(count).chain(network)
    }
}

/// One count a request goes against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Count {
    Account(Uuid),
    /// An IPv4 address or an IPv6 /64.
    Address(IpAddr),
    /// An IPv6 /48.
    Network(IpAddr),
}

impl Count {
    fn limit(&self, limits: &RequestLimits) -> u32 {
        match self {
            Self::Account(_) => limits.per_account,
            Self::Address(_) => limits.per_address,
            Self::Network(_) => limits.per_network,
        }
    }

    fn key(&self) -> String {
        match self {
            Self::Account(user_id) => format!("all:user:{}", user_id.simple()),
            Self::Address(address) => format!("all:addr:{address}"),
            Self::Network(network) => format!("all:net:{network}"),
        }
    }

    fn refusal(&self, limit: u32) -> String {
        match self {
            Self::Account(_) => format!("an account may make {limit} requests per minute"),
            Self::Address(_) => format!("{limit} requests per minute are allowed without signing in"),
            Self::Network(_) => format!("{limit} requests per minute are allowed without signing in from one IPv6 /48"),
        }
    }
}

/// What one subscriber holds: the address itself for IPv4, its /64 for IPv6.
fn network_of(address: IpAddr) -> IpAddr {
    match address.to_canonical() {
        IpAddr::V4(v4) => IpAddr::V4(v4),
        v6 => proxies::masked(v6, 64),
    }
}

/// Whether `path` is an RPC of the API.
fn is_rpc(path: &str) -> bool {
    path.strip_prefix("/tokuly.").is_some_and(|rest| rest.contains('/'))
}

/// The overall limits of [`crate::auth::auth_middleware`].
#[derive(Clone)]
pub(crate) struct RequestGate {
    /// Where requests are counted; `None` until limits are set. Its
    /// `TOKULY_RATE_LIMITS` switch is read on every request.
    pub(crate) limiter: Option<Cache>,
    pub(crate) limits: RequestLimits,
}

impl Default for RequestGate {
    fn default() -> Self {
        Self { limiter: None, limits: RequestLimits::NONE }
    }
}

impl RequestGate {
    /// The refusal of a request over its limit; `None` lets it through.
    pub(crate) async fn refusal(&self, path: &str, state: &AuthState, address: Option<IpAddr>) -> Option<Refused> {
        let cache = self.limiter.as_ref().filter(|cache| cache.enforces_rate_limits())?;
        if !is_rpc(path) {
            return None;
        }
        for count in Caller::of(state, address)?.counts() {
            let limit = count.limit(&self.limits);
            if limit == 0 {
                continue;
            }
            let status = match tokio::time::timeout(COUNT_TIMEOUT, count_per_minute(cache, &count.key(), limit)).await {
                Ok(Ok(status)) => status,
                Ok(Err(e)) => {
                    tracing::debug!(error = %e, "overall request limit not counted; request allowed");
                    return None;
                }
                Err(_) => {
                    tracing::debug!("overall request limit not counted in time; request allowed");
                    return None;
                }
            };
            if status.exceeded {
                return Some(Refused { error: AppError::rate_limited("RATE_LIMITED", count.refusal(limit)), status });
            }
        }
        None
    }
}

/// A request over its limit.
pub(crate) struct Refused {
    pub(crate) error: AppError,
    pub(crate) status: RateStatus,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{api_tokens::ApiToken, auth::AuthContext};

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn ipv6_addresses_count_per_64_and_ipv4_per_address() {
        assert_eq!(network_of(ip("2001:db8:1:2:aaaa::1")), ip("2001:db8:1:2::"));
        assert_eq!(network_of(ip("2001:db8:1:2:ffff:ffff:ffff:ffff")), network_of(ip("2001:db8:1:2::7")));
        assert_ne!(network_of(ip("2001:db8:1:3::1")), network_of(ip("2001:db8:1:2::1")));
        assert_eq!(network_of(ip("203.0.113.7")), ip("203.0.113.7"));
        assert_ne!(network_of(ip("203.0.113.7")), network_of(ip("203.0.113.8")));
        // An IPv4 client seen through an IPv6 socket is still that IPv4 address.
        assert_eq!(network_of(ip("::ffff:203.0.113.7")), ip("203.0.113.7"));
    }

    #[test]
    fn sessions_count_per_account_everyone_else_per_address_and_tokens_not_at_all() {
        let user = Uuid::now_v7();
        let session = |session_id| {
            AuthState::Authenticated(AuthContext {
                user_id: user,
                session_id,
                roles: Vec::new(),
                expires_at: Utc::now(),
                api_token: None,
                too_young: false,
            })
        };
        let address = Some(ip("2001:db8::5"));
        // Two sessions (two devices) of one account share its count.
        assert_eq!(Caller::of(&session(Uuid::now_v7()), address), Some(Caller::Account(user)));
        assert_eq!(Caller::of(&session(Uuid::now_v7()), None), Some(Caller::Account(user)));
        for state in [AuthState::Anonymous, AuthState::Invalid("TOKEN_EXPIRED")] {
            assert_eq!(Caller::of(&state, address), Some(Caller::Address(ip("2001:db8::"))));
            assert_eq!(Caller::of(&state, None), None);
        }
        let token = ApiToken {
            kind: crate::api_tokens::ApiTokenKind::Personal,
            id: Uuid::now_v7(),
            user_id: user,
            app_id: None,
            scopes: crate::api_tokens::Scopes::default(),
            expires_at: None,
            rate: crate::api_tokens::RateTier { name: "standard".into(), per_minute: 60 },
        };
        assert_eq!(Caller::of(&AuthState::Authenticated(AuthContext::from_api_token(token)), address), None);
    }

    #[test]
    fn ipv6_addresses_also_count_per_48() {
        let user = Uuid::now_v7();
        assert_eq!(Caller::Account(user).counts().collect::<Vec<_>>(), [Count::Account(user)]);
        let v4 = ip("203.0.113.7");
        assert_eq!(Caller::Address(v4).counts().collect::<Vec<_>>(), [Count::Address(v4)]);
        let v6 = ip("2001:db8:1:2::");
        assert_eq!(
            Caller::Address(v6).counts().collect::<Vec<_>>(),
            [Count::Address(v6), Count::Network(ip("2001:db8:1::"))]
        );
    }

    #[test]
    fn only_rpcs_are_counted() {
        assert!(is_rpc("/tokuly.notes.v1.NoteService/GetNote"));
        assert!(is_rpc("/tokuly.x.v1.Unknown/Method"));
        for other in ["/healthz", "/readyz", "/oauth/token", "/media/link-image/abc", "/tokuly.", "/"] {
            assert!(!is_rpc(other), "{other}");
        }
    }
}
