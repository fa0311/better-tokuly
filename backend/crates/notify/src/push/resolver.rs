//! Name resolution for Web Push endpoints that only yields public addresses.
//!
//! An endpoint URL comes from a client, and the server POSTs to it. The
//! endpoint policy vets the URL (`webpush.rs`), but a name is only a name: a
//! host allowed by `*` (or a listed one whose DNS someone controls) can
//! resolve to a loopback, private or link-local address and aim the request
//! at an internal service. This resolver refuses such addresses at the
//! moment of connecting, so a second, different DNS answer cannot slip
//! through either.
//!
//! The same rules as the media domain's link-preview guard
//! (`tokuly_media::link_preview::guard`), which this crate may not depend on.
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    let special = a == 0                                  // "this network"
        || a == 10                                        // private
        || a == 127                                       // loopback
        || (a == 100 && (64..128).contains(&b))           // carrier-grade NAT
        || (a == 169 && b == 254)                         // link-local, cloud metadata
        || (a == 172 && (16..32).contains(&b))            // private
        || (a == 192 && b == 168)                         // private
        || (a == 192 && b == 0 && (c == 0 || c == 2))     // protocol assignments, documentation
        || (a == 192 && b == 88 && c == 99)               // 6to4 relay anycast
        || (a == 198 && (18..20).contains(&b))            // benchmarking
        || (a == 198 && b == 51 && c == 100)              // documentation
        || (a == 203 && b == 0 && c == 113)               // documentation
        || a >= 224; // multicast, reserved, broadcast
    !special
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    // Forms that carry an IPv4 address are judged by that address:
    // IPv4-mapped, IPv4-compatible, NAT64 (64:ff9b::/96) and 6to4 (2002::/16).
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    let octets = ip.octets();
    let embedded = |at: usize| Ipv4Addr::new(octets[at], octets[at + 1], octets[at + 2], octets[at + 3]);
    if segments[..6] == [0, 0, 0, 0, 0, 0] && !ip.is_unspecified() && !ip.is_loopback() {
        return is_public_v4(embedded(12));
    }
    if segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return is_public_v4(embedded(12));
    }
    if segments[0] == 0x2002 {
        return is_public_v4(embedded(2));
    }
    let special = ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
        || (segments[0] & 0xffc0) == 0xfec0               // site-local (deprecated)
        || (segments[0] == 0x2001 && segments[1] == 0x0db8) // documentation
        || (segments[0] == 0x0100 && segments[1..4] == [0, 0, 0]); // discard-only
    !special
}

/// Whether `ip` is a globally routable unicast address.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

/// A resolver that drops every address that is not public; a name left with
/// none fails to resolve (the push is then retried and, in the end, lost).
#[derive(Debug, Clone, Copy, Default)]
pub struct PublicOnly;

impl PublicOnly {
    pub fn shared() -> Arc<dyn Resolve> {
        Arc::new(Self)
    }
}

impl Resolve for PublicOnly {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let resolved: Vec<SocketAddr> = tokio::net::lookup_host((name.as_str(), 0)).await?.collect();
            let public: Vec<SocketAddr> = resolved.into_iter().filter(|address| is_public(address.ip())).collect();
            if public.is_empty() {
                return Err(format!("{} has no public address", name.as_str()).into());
            }
            Ok(Box::new(public.into_iter()) as Addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use super::*;

    #[test]
    fn internal_addresses_are_not_public() {
        for ip in [
            "0.0.0.0",
            "10.1.2.3",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "192.168.1.1",
            "192.0.2.1",
            "198.18.0.1",
            "224.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "fc00::1",
            "fd12::1",
            "fe80::1",
            "ff02::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "64:ff9b::a9fe:a9fe",
            "2002:7f00:1::1",
            "::127.0.0.1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "142.250.196.110", "2001:4860:4860::8888", "::ffff:8.8.8.8"] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[tokio::test]
    async fn a_name_that_resolves_inside_the_network_does_not_resolve() {
        let refused = PublicOnly.resolve(Name::from_str("localhost").unwrap()).await;
        assert!(refused.is_err());
    }
}
