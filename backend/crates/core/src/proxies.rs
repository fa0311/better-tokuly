//! Which address a request really comes from.
//!
//! Requests usually arrive through the edge proxy, which names the client in
//! `X-Forwarded-For`. The header is only believed when the TCP peer is one of
//! our proxies: anyone else could write whatever they like into it, to dodge
//! a per-address limit or to exhaust somebody else's.
use std::{
    net::{IpAddr, SocketAddr},
    str::FromStr,
};

/// Default for the trusted-proxy settings: loopback and the private ranges,
/// which covers an edge proxy on the same host or container network.
pub const DEFAULT_TRUSTED_PROXIES: &str = "127.0.0.0/8,::1,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16,fc00::/7";

/// An address range in CIDR notation (`10.0.0.0/8`, `fc00::/7`) or a single
/// address (`203.0.113.7`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Network {
    /// The range's first address (host bits cleared).
    base: IpAddr,
    prefix: u8,
}

impl Network {
    fn contains(&self, address: IpAddr) -> bool {
        let address = address.to_canonical();
        address.is_ipv4() == self.base.is_ipv4() && masked(address, self.prefix) == self.base
    }
}

impl FromStr for Network {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let (address, prefix) = match text.split_once('/') {
            Some((address, prefix)) => (address, Some(prefix)),
            None => (text, None),
        };
        let address: IpAddr = address.trim().parse().map_err(|_| format!("`{text}` is not an IP address or CIDR range"))?;
        let address = address.to_canonical();
        let bits = if address.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(prefix) => prefix.trim().parse::<u8>().ok().filter(|prefix| *prefix <= bits).ok_or_else(|| format!("`{text}` has an invalid prefix length"))?,
            None => bits,
        };
        Ok(Self { base: masked(address, prefix), prefix })
    }
}

/// `address` with everything after the first `prefix` bits cleared.
pub fn masked(address: IpAddr, prefix: u8) -> IpAddr {
    match address {
        IpAddr::V4(v4) => IpAddr::V4((u32::from(v4) & u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0)).into()),
        IpAddr::V6(v6) => IpAddr::V6((u128::from(v6) & u128::MAX.checked_shl(128 - u32::from(prefix)).unwrap_or(0)).into()),
    }
}

/// The proxies whose `X-Forwarded-For` header is believed.
#[derive(Debug)]
pub struct TrustedProxies(Vec<Network>);

impl TrustedProxies {
    /// Parses addresses and CIDR ranges; empty entries are skipped.
    pub fn parse(entries: &[String]) -> Result<Self, String> {
        entries.iter().map(|entry| entry.trim()).filter(|entry| !entry.is_empty()).map(Network::from_str).collect::<Result<_, _>>().map(Self)
    }

    fn trusts(&self, address: IpAddr) -> bool {
        self.0.iter().any(|network| network.contains(address))
    }

    /// The client behind a connection from `peer` carrying this
    /// `X-Forwarded-For` header.
    ///
    /// Each proxy appends the address it received the request from, so the
    /// chain is read from the right: as long as the hop that vouches for an
    /// entry is ours, the entry is believed. The first address that is not
    /// one of our proxies is the client; whatever stands left of it was
    /// supplied by that client and means nothing.
    pub fn client_ip(&self, peer: IpAddr, forwarded_for: Option<&str>) -> IpAddr {
        let mut client = peer.to_canonical();
        for hop in forwarded_for.unwrap_or_default().rsplit(',') {
            if !self.trusts(client) {
                break;
            }
            let hop = hop.trim();
            // Some proxies add the port.
            let Ok(address) = hop.parse::<IpAddr>().or_else(|_| hop.parse::<SocketAddr>().map(|socket| socket.ip())) else { break };
            client = address.to_canonical();
        }
        client
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    fn proxies(entries: &[&str]) -> TrustedProxies {
        TrustedProxies::parse(&entries.iter().map(|entry| entry.to_string()).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn networks_match_their_range_only() {
        let private: Network = "10.0.0.0/8".parse().unwrap();
        assert!(private.contains(ip("10.1.2.3")) && !private.contains(ip("11.0.0.1")));
        // Host bits in the notation are ignored; a bare address is a range of one.
        assert_eq!("10.9.9.9/8".parse::<Network>().unwrap(), private);
        let single: Network = "203.0.113.7".parse().unwrap();
        assert!(single.contains(ip("203.0.113.7")) && !single.contains(ip("203.0.113.8")));

        let unique_local: Network = "fc00::/7".parse().unwrap();
        assert!(unique_local.contains(ip("fd12:3456::1")) && !unique_local.contains(ip("2001:db8::1")));
        // Families never match each other, but an IPv4-mapped address is IPv4.
        assert!(!private.contains(ip("::1")));
        assert!(private.contains(ip("::ffff:10.1.2.3")));
        assert!("0.0.0.0/0".parse::<Network>().unwrap().contains(ip("8.8.8.8")));

        for invalid in ["", "ten", "10.0.0.0/33", "fc00::/129", "10.0.0.0/x"] {
            assert!(invalid.parse::<Network>().is_err(), "{invalid}");
        }
        assert!(TrustedProxies::parse(&["10.0.0.0/8".into(), "nonsense".into()]).is_err());
        assert!(TrustedProxies::parse(&[String::new()]).unwrap().0.is_empty());
    }

    #[test]
    fn direct_clients_cannot_pick_their_address() {
        let trusted = proxies(&["10.0.0.0/8"]);
        assert_eq!(trusted.client_ip(ip("198.51.100.4"), None), ip("198.51.100.4"));
        assert_eq!(trusted.client_ip(ip("198.51.100.4"), Some("203.0.113.7")), ip("198.51.100.4"));
        assert_eq!(proxies(&[]).client_ip(ip("10.0.0.2"), Some("203.0.113.7")), ip("10.0.0.2"));
    }

    #[test]
    fn the_client_is_the_first_address_our_proxies_do_not_vouch_for() {
        let trusted = proxies(&["10.0.0.0/8", "192.0.2.0/24"]);
        let proxy = ip("10.0.0.2");
        assert_eq!(trusted.client_ip(proxy, Some("203.0.113.7")), ip("203.0.113.7"));
        // Entries left of the client were written by the client.
        assert_eq!(trusted.client_ip(proxy, Some("1.1.1.1, 203.0.113.7")), ip("203.0.113.7"));
        // Two of our proxies in a row (a CDN in front of the edge).
        assert_eq!(trusted.client_ip(proxy, Some("1.1.1.1, 203.0.113.7, 192.0.2.10")), ip("203.0.113.7"));
        // A client inside our network.
        assert_eq!(trusted.client_ip(proxy, Some("10.0.0.99")), ip("10.0.0.99"));
        // No or unreadable header: the proxy itself is all we know.
        assert_eq!(trusted.client_ip(proxy, None), proxy);
        assert_eq!(trusted.client_ip(proxy, Some("unknown")), proxy);
        assert_eq!(trusted.client_ip(proxy, Some("203.0.113.7:51234")), ip("203.0.113.7"));
        assert_eq!(trusted.client_ip(ip("::ffff:10.0.0.2"), Some("2001:db8::5")), ip("2001:db8::5"));
    }
}
