//! The overall request limits of the auth middleware against a real Valkey
//! (`TOKULY_VALKEY_URL`, as exported by `tools/dev/infra.sh env`), and what
//! the middleware does while Valkey does not answer.
use std::{
    net::{IpAddr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokuly_core::{
    api_access,
    auth::{AuthLayer, Tokens, auth_middleware},
    cache::Cache,
    keys::Keys,
    proxies::TrustedProxies,
    request_limits::RequestLimits,
};
use tower::ServiceExt as _;
use uuid::Uuid;

const SECRET: &str = "0123456789abcdef0123456789abcdef";
const GET_NOTE: &str = "/tokuly.notes.v1.NoteService/GetNote";

fn tokens() -> Tokens {
    Tokens::new(&Keys::new(SECRET)).unwrap()
}

async fn valkey() -> Cache {
    let url = std::env::var("TOKULY_VALKEY_URL").unwrap_or_else(|_| "redis://localhost:6379".to_owned());
    Cache::connect(&url).await.expect("Valkey is reachable")
}

fn app(tokens: Tokens, limits: RequestLimits, cache: Cache) -> axum::Router {
    let proxies = TrustedProxies::parse(&["10.0.0.0/8".to_owned()]).unwrap();
    let layer = AuthLayer::new(tokens, proxies).with_request_limits(limits, cache);
    axum::Router::new().fallback(|| async { "ok" }).layer(axum::middleware::from_fn_with_state(layer, auth_middleware))
}

/// A caller: its address, and the session it presents (if any).
struct Caller<'a> {
    peer: IpAddr,
    forwarded_for: Option<&'a str>,
    bearer: Option<&'a str>,
}

impl<'a> Caller<'a> {
    fn anonymous(peer: IpAddr) -> Self {
        Self { peer, forwarded_for: None, bearer: None }
    }

    fn signed_in(peer: IpAddr, token: &'a str) -> Self {
        Self { peer, forwarded_for: None, bearer: Some(token) }
    }
}

/// One call: status, error reason, and the `retry-after` / `x-ratelimit-reset` headers.
async fn call(app: &axum::Router, path: &str, caller: &Caller<'_>) -> (u16, String, Option<u32>, Option<u32>) {
    let mut request = http::Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .extension(axum::extract::ConnectInfo(SocketAddr::new(caller.peer, 40_000)));
    if let Some(forwarded_for) = caller.forwarded_for {
        request = request.header("x-forwarded-for", forwarded_for);
    }
    if let Some(bearer) = caller.bearer {
        request = request.header("authorization", format!("Bearer {bearer}"));
    }
    let response = app.clone().oneshot(request.body(axum::body::Body::empty()).unwrap()).await.unwrap();
    let header = |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
    let number = |name: &str| header(name).and_then(|v| v.parse::<u32>().ok());
    (
        response.status().as_u16(),
        header("x-tokuly-error").unwrap_or_default(),
        number("retry-after"),
        number("x-ratelimit-reset"),
    )
}

/// An address nobody else uses in this run (documentation ranges, random host part).
fn fresh_v4() -> IpAddr {
    let n = Uuid::now_v7().as_u128();
    IpAddr::from([198, 18 + (n % 2) as u8, (n >> 8) as u8, (n >> 16) as u8])
}

fn fresh_v6_network() -> u128 {
    // 2001:db8:<random 32 bits>::/64
    (0x2001_0db8u128 << 96) | (((Uuid::now_v7().as_u128() >> 32) as u32 as u128) << 64)
}

fn v6(network: u128, host: u64) -> IpAddr {
    IpAddr::from((network | u128::from(host)).to_be_bytes())
}

#[tokio::test]
async fn an_account_is_counted_over_all_its_sessions() {
    let tokens = tokens();
    let app = app(tokens.clone(), RequestLimits { per_account: 3, per_address: 0 }, valkey().await);
    let user = Uuid::now_v7();
    let (phone, _) = tokens.issue(user, Uuid::now_v7(), &[]).unwrap();
    let (laptop, _) = tokens.issue(user, Uuid::now_v7(), &[]).unwrap();
    let (someone_else, _) = tokens.issue(Uuid::now_v7(), Uuid::now_v7(), &[]).unwrap();

    for (token, address) in [(&phone, fresh_v4()), (&laptop, fresh_v4()), (&phone, fresh_v4())] {
        assert_eq!(call(&app, GET_NOTE, &Caller::signed_in(address, token)).await.0, 200);
    }
    let (status, reason, retry_after, reset) = call(&app, GET_NOTE, &Caller::signed_in(fresh_v4(), &laptop)).await;
    assert_eq!((status, reason.as_str()), (429, "RATE_LIMITED"));
    assert!(retry_after.is_some_and(|seconds| (1..=60).contains(&seconds)), "retry-after: {retry_after:?}");
    assert_eq!(reset, retry_after);
    // Other accounts, and every RPC of this one, are counted on their own.
    assert_eq!(call(&app, GET_NOTE, &Caller::signed_in(fresh_v4(), &someone_else)).await.0, 200);
    assert_eq!(call(&app, "/tokuly.chat.v1.ConversationService/ListConversations", &Caller::signed_in(fresh_v4(), &phone)).await.0, 429);
}

#[tokio::test]
async fn callers_without_an_account_are_counted_per_address_and_ipv6_per_64() {
    let app = app(tokens(), RequestLimits { per_account: 0, per_address: 2 }, valkey().await);
    let network = fresh_v6_network();
    // Hopping between addresses of one /64 does not start a new count.
    for host in [1, 2] {
        assert_eq!(call(&app, GET_NOTE, &Caller::anonymous(v6(network, host))).await.0, 200);
    }
    let (status, reason, retry_after, _) = call(&app, GET_NOTE, &Caller::anonymous(v6(network, 0xffff))).await;
    assert_eq!((status, reason.as_str()), (429, "RATE_LIMITED"));
    assert!(retry_after.is_some());
    // A token that is no longer valid counts as no account at all.
    let stale = Caller { bearer: Some("not.a.token"), ..Caller::anonymous(v6(network, 3)) };
    assert_eq!(call(&app, GET_NOTE, &stale).await.0, 429);
    // The next /64 has its own count.
    assert_eq!(call(&app, GET_NOTE, &Caller::anonymous(v6(network + (1 << 64), 1))).await.0, 200);

    // Behind our proxy, the client it names is counted, not the proxy.
    let client = fresh_v4().to_string();
    for _ in 0..2 {
        let caller = Caller { forwarded_for: Some(&client), ..Caller::anonymous("10.0.0.2".parse().unwrap()) };
        assert_eq!(call(&app, GET_NOTE, &caller).await.0, 200);
    }
    let next = fresh_v4().to_string();
    let caller = Caller { forwarded_for: Some(&next), ..Caller::anonymous("10.0.0.2".parse().unwrap()) };
    assert_eq!(call(&app, GET_NOTE, &caller).await.0, 200, "another client behind the same proxy");
}

/// A proxy may add its `X-Forwarded-For` as a field of its own after the
/// one the client sent (HAProxy does): the client is still the one our proxy
/// names, not the one the client claims in the first field.
#[tokio::test]
async fn a_forwarded_for_field_of_the_client_does_not_pick_the_address() {
    let app = app(tokens(), RequestLimits { per_account: 0, per_address: 2 }, valkey().await);
    let client = fresh_v4().to_string();
    let call = |claimed: String| {
        let request = http::Request::post(GET_NOTE)
            .extension(axum::extract::ConnectInfo(SocketAddr::new("10.0.0.2".parse().unwrap(), 40_000)))
            .header("x-forwarded-for", claimed)
            .header("x-forwarded-for", &client)
            .body(axum::body::Body::empty())
            .unwrap();
        app.clone().oneshot(request)
    };
    for _ in 0..2 {
        assert_eq!(call(fresh_v4().to_string()).await.unwrap().status(), 200);
    }
    assert_eq!(call(fresh_v4().to_string()).await.unwrap().status(), 429, "a new claimed address does not start a new count");
}

/// The overall limit is the floor under every RPC: whatever a handler
/// forgot, a caller without an account is limited per address on each RPC
/// of the API, listed or not.
#[tokio::test]
async fn every_rpc_callable_without_an_account_is_limited_per_address() {
    let app = app(tokens(), RequestLimits { per_account: 0, per_address: 1 }, valkey().await);
    let paths: Vec<String> = api_access::classified()
        .map(|(path, _)| path)
        .chain(["/tokuly.unlisted.v1.NewService/NewMethod".to_owned()])
        .collect();
    assert!(paths.len() > 100, "the table lists the API");
    let mut unlimited = Vec::new();
    for path in &paths {
        let caller = Caller::anonymous(v6(fresh_v6_network(), 1));
        assert_eq!(call(&app, path, &caller).await.0, 200, "{path}");
        if call(&app, path, &caller).await.0 != 429 {
            unlimited.push(path.as_str());
        }
    }
    assert!(unlimited.is_empty(), "callable without an account and not limited per address: {unlimited:?}");
}

#[tokio::test]
async fn what_is_not_counted() {
    let tokens = tokens();
    let cache = valkey().await;
    let app = app(tokens.clone(), RequestLimits { per_account: 1, per_address: 1 }, cache.clone());
    let address = fresh_v4();
    // Plain HTTP routes (probes, OAuth, pictures) are limited by their domains.
    for path in ["/healthz", "/oauth/token", "/media/link-image/x"] {
        for _ in 0..3 {
            assert_eq!(call(&app, path, &Caller::anonymous(address)).await.0, 200, "{path}");
        }
    }
    // A caller whose address is unknown (served without peer information).
    let unknown = app.clone();
    for _ in 0..3 {
        let request = http::Request::post(GET_NOTE).body(axum::body::Body::empty()).unwrap();
        assert_eq!(unknown.clone().oneshot(request).await.unwrap().status(), 200);
    }
    // 0 is no limit.
    let unlimited = self::app(tokens.clone(), RequestLimits::NONE, cache.clone());
    let (token, _) = tokens.issue(Uuid::now_v7(), Uuid::now_v7(), &[]).unwrap();
    for _ in 0..3 {
        assert_eq!(call(&unlimited, GET_NOTE, &Caller::signed_in(address, &token)).await.0, 200);
        assert_eq!(call(&unlimited, GET_NOTE, &Caller::anonymous(address)).await.0, 200);
    }
}

#[tokio::test]
async fn a_deployment_without_rate_limits_is_not_counted() {
    // TOKULY_RATE_LIMITS=false (the browser tests, the dev setup); a cache of its own, as the switch is shared by clones.
    let cache = valkey().await.with_rate_limits(false);
    let app = app(tokens(), RequestLimits { per_account: 1, per_address: 1 }, cache.clone());
    let address = fresh_v4();
    for _ in 0..3 {
        assert_eq!(call(&app, GET_NOTE, &Caller::anonymous(address)).await.0, 200);
    }
    // Switched on while running (the console), the next request is counted.
    cache.set_rate_limits(true);
    assert_eq!(call(&app, GET_NOTE, &Caller::anonymous(address)).await.0, 200);
    assert_eq!(call(&app, GET_NOTE, &Caller::anonymous(address)).await.0, 429);
}

// ───────────────────────────── Valkey not answering ─────────────────────────────

/// A Valkey that answers `+OK` while connecting and then stops answering
/// (a server that hangs, or a network that drops everything).
struct SilentValkey {
    url: String,
    silent: Arc<AtomicBool>,
}

impl SilentValkey {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("redis://{}", listener.local_addr().unwrap());
        let silent = Arc::new(AtomicBool::new(false));
        let quiet = silent.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let quiet = quiet.clone();
                tokio::spawn(async move {
                    let mut buffer = [0u8; 4096];
                    while let Ok(read) = socket.read(&mut buffer).await {
                        if read == 0 {
                            break;
                        }
                        if quiet.load(Ordering::Relaxed) {
                            continue;
                        }
                        // One `+OK` per command (each starts with `*` at the start of a line).
                        let commands = buffer[..read]
                            .iter()
                            .enumerate()
                            .filter(|(at, byte)| **byte == b'*' && (*at == 0 || buffer[at - 1] == b'\n'))
                            .count();
                        let _ = socket.write_all(&b"+OK\r\n".repeat(commands.max(1))).await;
                    }
                });
            }
        });
        Self { url, silent }
    }

    async fn cache(&self) -> Cache {
        let cache = Cache::connect(&self.url).await.expect("the fake Valkey accepts connections");
        self.silent.store(true, Ordering::Relaxed);
        cache
    }
}

#[tokio::test]
async fn while_valkey_does_not_answer_requests_go_through_but_staff_sessions_do_not() {
    let valkey = SilentValkey::start().await;
    let cache = valkey.cache().await;
    let tokens = tokens().with_revocations(cache.clone());
    let app = app(tokens.clone(), RequestLimits { per_account: 1, per_address: 1 }, cache);
    let (member, _) = tokens.issue(Uuid::now_v7(), Uuid::now_v7(), &[]).unwrap();
    let (moderator, _) = tokens.issue(Uuid::now_v7(), Uuid::now_v7(), &["moderator".to_owned()]).unwrap();
    let address = fresh_v4();

    let started = std::time::Instant::now();
    // The overall limit cannot count: nobody is held up by it.
    for _ in 0..3 {
        assert_eq!(call(&app, GET_NOTE, &Caller::anonymous(address)).await.0, 200);
        assert_eq!(call(&app, GET_NOTE, &Caller::signed_in(address, &member)).await.0, 200);
    }
    // Whether a staff session was revoked cannot be known: its powers wait
    // until Valkey answers again (unavailable, so the app keeps the session and retries).
    let (status, reason, _, _) = call(&app, GET_NOTE, &Caller::signed_in(address, &moderator)).await;
    assert_eq!((status, reason.as_str()), (503, "SESSION_UNVERIFIABLE"));
    assert!(started.elapsed() < std::time::Duration::from_secs(3), "every lookup gives up quickly: {:?}", started.elapsed());
}
