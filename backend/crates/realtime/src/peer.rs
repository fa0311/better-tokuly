//! Where a connection really comes from, and how many one address may hold.
//!
//! The gateway faces the internet, so a single address must not be able to
//! take every connection slot. That needs the client's true address:
//!
//! - WebTransport clients connect directly; QUIC proves their address.
//! - WebSocket clients usually arrive through the edge proxy, which names
//!   them in `X-Forwarded-For`; `tokuly_core::proxies` decides whether to
//!   believe it.
use std::{net::IpAddr, sync::Arc};

use dashmap::{DashMap, Entry};
use tokuly_core::proxies::masked;

/// What the limit counts as "one address": an IPv4 address, or an IPv6 /64
/// (a single subscriber or device can use any address within its /64).
fn bucket_of(address: IpAddr) -> IpAddr {
    match address.to_canonical() {
        v4 @ IpAddr::V4(_) => v4,
        v6 @ IpAddr::V6(_) => masked(v6, 64),
    }
}

/// Counts open connections per client address and refuses those over the limit.
pub(crate) struct IpLimiter {
    /// Connections one address may hold; 0 = unlimited.
    max: usize,
    open: DashMap<IpAddr, usize>,
}

impl IpLimiter {
    pub(crate) fn new(max: usize) -> Arc<Self> {
        Arc::new(Self { max, open: DashMap::new() })
    }

    /// Whether `address` already holds as many connections as it may.
    pub(crate) fn is_full(&self, address: IpAddr) -> bool {
        self.max > 0 && self.open.get(&bucket_of(address)).is_some_and(|open| *open >= self.max)
    }

    /// Reserves a connection slot for `address`; `None` if it has none left.
    /// The slot is freed when the permit is dropped.
    pub(crate) fn admit(self: &Arc<Self>, address: IpAddr) -> Option<IpPermit> {
        if self.max == 0 {
            return Some(IpPermit { limiter: self.clone(), bucket: None });
        }
        let bucket = bucket_of(address);
        let mut open = self.open.entry(bucket).or_insert(0);
        if *open >= self.max {
            return None;
        }
        *open += 1;
        Some(IpPermit { limiter: self.clone(), bucket: Some(bucket) })
    }
}

/// One address's claim on a connection slot.
pub(crate) struct IpPermit {
    limiter: Arc<IpLimiter>,
    bucket: Option<IpAddr>,
}

impl Drop for IpPermit {
    fn drop(&mut self) {
        let Some(bucket) = self.bucket else { return };
        if let Entry::Occupied(mut open) = self.limiter.open.entry(bucket) {
            *open.get_mut() -= 1;
            if *open.get() == 0 {
                open.remove();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn an_address_gets_only_so_many_connections() {
        let limiter = IpLimiter::new(2);
        let (here, there) = (ip("203.0.113.7"), ip("203.0.113.8"));
        let first = limiter.admit(here).expect("first");
        let _second = limiter.admit(here).expect("second");
        assert!(limiter.is_full(here) && limiter.admit(here).is_none());
        // Other addresses are unaffected.
        assert!(!limiter.is_full(there));
        let _elsewhere = limiter.admit(there).expect("another address");

        // A closed connection frees its slot.
        drop(first);
        assert!(!limiter.is_full(here));
        let _third = limiter.admit(here).expect("slot freed");
    }

    #[test]
    fn nothing_is_remembered_about_addresses_without_connections() {
        let limiter = IpLimiter::new(2);
        drop(limiter.admit(ip("203.0.113.7")));
        assert!(limiter.open.is_empty());
    }

    #[test]
    fn an_ipv6_subscriber_is_one_address() {
        let limiter = IpLimiter::new(1);
        let _held = limiter.admit(ip("2001:db8:1:2::1")).expect("first");
        assert!(limiter.admit(ip("2001:db8:1:2:ffff::9")).is_none(), "same /64");
        assert!(limiter.admit(ip("2001:db8:1:3::1")).is_some(), "another /64");
    }

    #[test]
    fn a_limit_of_zero_admits_everyone() {
        let limiter = IpLimiter::new(0);
        let permits: Vec<_> = (0..100).map(|_| limiter.admit(ip("203.0.113.7")).expect("unlimited")).collect();
        assert!(!limiter.is_full(ip("203.0.113.7")) && limiter.open.is_empty());
        drop(permits);
    }
}
