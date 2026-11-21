//! SSRF protection for the link preview fetcher.
//!
//! The fetcher requests URLs chosen by users, from inside our network. It
//! must therefore never reach anything that is not the public internet:
//! loopback, private ranges, link-local addresses (cloud metadata services
//! live there), carrier NAT, multicast, and every IPv6 form that embeds such
//! an IPv4 address.
//!
//! Two checks enforce this, on every redirect hop:
//! - [`check_url`] validates the URL itself (scheme, port, literal IPs);
//! - [`GuardedResolver`] validates what a host name *resolves to*, at the
//!   moment the connection is made. Checking a name earlier and connecting
//!   later would let a hostile DNS server answer differently the second
//!   time (DNS rebinding).
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use url::{Host, Url};

/// What the fetcher may reach.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Policy {
    /// Permit loopback addresses and any port. For integration tests against
    /// a local server; never set in production code paths.
    pub allow_loopback: bool,
}

/// Why a URL (or an address it leads to) was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Scheme,
    Credentials,
    Port,
    NoHost,
    Address(IpAddr),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Scheme => write!(f, "only http and https URLs are fetched"),
            Self::Credentials => write!(f, "URLs with credentials are not fetched"),
            Self::Port => write!(f, "only the default ports are fetched"),
            Self::NoHost => write!(f, "the URL has no host"),
            Self::Address(ip) => write!(f, "{ip} is not a public address"),
        }
    }
}

impl std::error::Error for Refusal {}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    let special = a == 0                                  // "this network"
        || a == 10                                        // private
        || a == 127                                       // loopback
        || (a == 100 && (64..128).contains(&b))           // carrier-grade NAT
        || (a == 169 && b == 254)                         // link-local, cloud metadata
        || (a == 172 && (16..32).contains(&b))            // private
        || (a == 192 && b == 168)                         // private
        || (a == 192 && b == 0 && c == 0)                 // IETF protocol assignments
        || (a == 192 && b == 0 && c == 2)                 // documentation
        || (a == 192 && b == 88 && c == 99)               // 6to4 relay anycast
        || (a == 198 && (18..20).contains(&b))            // benchmarking
        || (a == 198 && b == 51 && c == 100)              // documentation
        || (a == 203 && b == 0 && c == 113)               // documentation
        || a >= 224; // multicast, reserved, broadcast
    !special
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    // Forms that carry an IPv4 address are judged by that address.
    if let Some(v4) = embedded_v4(ip) {
        return is_public_v4(v4);
    }
    let segments = ip.segments();
    let special = ip.is_unspecified()
        || ip.is_loopback()
        || (segments[0] & 0xfe00) == 0xfc00               // unique local (fc00::/7)
        || (segments[0] & 0xffc0) == 0xfe80               // link-local (fe80::/10)
        || (segments[0] & 0xffc0) == 0xfec0               // site-local (deprecated)
        || (segments[0] & 0xff00) == 0xff00               // multicast
        || (segments[0] == 0x2001 && segments[1] == 0x0db8) // documentation
        || (segments[0] == 0x2001 && segments[1] == 0)    // Teredo tunnels
        || (segments[0] == 0x0100 && segments[1..4] == [0, 0, 0]); // discard-only
    !special
}

/// The IPv4 address inside an IPv4-mapped (`::ffff:a.b.c.d`),
/// IPv4-compatible (`::a.b.c.d`), NAT64 (`64:ff9b::/96`) or 6to4
/// (`2002::/16`) IPv6 address.
fn embedded_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let segments = ip.segments();
    let low = || Ipv4Addr::new((segments[6] >> 8) as u8, segments[6] as u8, (segments[7] >> 8) as u8, segments[7] as u8);
    match segments {
        [0, 0, 0, 0, 0, 0xffff, _, _] => Some(low()),
        [0, 0, 0, 0, 0, 0, high, _] if high != 0 => Some(low()),
        [0x0064, 0xff9b, 0, 0, 0, 0, _, _] => Some(low()),
        [0x2002, high, low_bits, ..] => Some(Ipv4Addr::new((high >> 8) as u8, high as u8, (low_bits >> 8) as u8, low_bits as u8)),
        _ => None,
    }
}

/// Whether `ip` is a globally routable unicast address.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

impl Policy {
    pub fn permits(self, ip: IpAddr) -> bool {
        is_public(ip) || (self.allow_loopback && ip.to_canonical().is_loopback())
    }
}

/// Validates a URL before it is requested: http(s) only, no credentials,
/// default ports only, and no literal address outside the public internet.
/// Host names are checked when they are resolved ([`GuardedResolver`]).
pub fn check_url(url: &Url, policy: Policy) -> Result<(), Refusal> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Refusal::Scheme);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Refusal::Credentials);
    }
    // `port()` is `None` for the scheme's default port.
    if url.port().is_some() && !policy.allow_loopback {
        return Err(Refusal::Port);
    }
    let address = match url.host() {
        None => return Err(Refusal::NoHost),
        Some(Host::Domain(_)) => return Ok(()),
        Some(Host::Ipv4(v4)) => IpAddr::V4(v4),
        Some(Host::Ipv6(v6)) => IpAddr::V6(v6),
    };
    if policy.permits(address) { Ok(()) } else { Err(Refusal::Address(address)) }
}

/// DNS resolver that only ever yields addresses the policy permits. A name
/// that resolves to nothing permitted fails to resolve.
#[derive(Debug, Clone, Copy)]
pub struct GuardedResolver {
    pub policy: Policy,
}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let policy = self.policy;
        Box::pin(async move {
            let resolved: Vec<SocketAddr> = tokio::net::lookup_host((name.as_str(), 0)).await?.collect();
            let permitted: Vec<SocketAddr> = resolved.iter().copied().filter(|address| policy.permits(address.ip())).collect();
            match (permitted.is_empty(), resolved.first()) {
                (false, _) => Ok(Box::new(permitted.into_iter()) as Addrs),
                (true, Some(refused)) => Err(Box::new(Refusal::Address(refused.ip())) as Box<dyn std::error::Error + Send + Sync>),
                (true, None) => Err(format!("{} does not resolve", name.as_str()).into()),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn public(ip: &str) -> bool {
        is_public(ip.parse().unwrap())
    }

    #[test]
    fn private_and_special_ipv4_ranges_are_not_public() {
        for ip in [
            "0.0.0.0",
            "0.1.2.3",
            "10.0.0.1",
            "10.255.255.255",
            "100.64.0.1",
            "100.127.255.255",
            "127.0.0.1",
            "127.255.255.254",
            "169.254.169.254", // cloud metadata service
            "169.254.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.8",
            "192.0.2.1",
            "192.88.99.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.19.255.255",
            "198.51.100.7",
            "203.0.113.9",
            "224.0.0.1",
            "239.255.255.250",
            "240.0.0.1",
            "255.255.255.255",
        ] {
            assert!(!public(ip), "{ip} must be refused");
        }
        for ip in ["1.1.1.1", "8.8.8.8", "100.63.255.255", "100.128.0.1", "172.15.255.255", "172.32.0.1", "192.167.1.1", "198.17.0.1", "198.20.0.1", "223.255.255.255"] {
            assert!(public(ip), "{ip} is public");
        }
    }

    #[test]
    fn private_and_special_ipv6_ranges_are_not_public() {
        for ip in [
            "::",
            "::1",
            "fc00::1",
            "fd00:ec2::254", // AWS metadata over IPv6
            "fdff:ffff::1",
            "fe80::1",
            "febf::1",
            "fec0::1",
            "ff02::1",
            "2001:db8::1",
            "2001:0:4136:e378:8000:63bf:3fff:fdd2", // Teredo
            "100::1",
        ] {
            assert!(!public(ip), "{ip} must be refused");
        }
        for ip in ["2606:4700:4700::1111", "2001:4860:4860::8888", "2a00:1450:4001:81b::200e"] {
            assert!(public(ip), "{ip} is public");
        }
    }

    #[test]
    fn ipv6_forms_embedding_ipv4_are_judged_by_the_ipv4_address() {
        for ip in [
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::ffff:169.254.169.254",
            "::127.0.0.1",
            "::10.1.2.3",
            "64:ff9b::7f00:1",        // NAT64 of 127.0.0.1
            "64:ff9b::a9fe:a9fe",     // NAT64 of 169.254.169.254
            "2002:7f00:1::1",         // 6to4 of 127.0.0.1
            "2002:c0a8:101::1",       // 6to4 of 192.168.1.1
            "2002:a9fe:a9fe::",       // 6to4 of 169.254.169.254
        ] {
            assert!(!public(ip), "{ip} must be refused");
        }
        for ip in ["::ffff:8.8.8.8", "64:ff9b::808:808", "2002:808:808::1"] {
            assert!(public(ip), "{ip} is public");
        }
    }

    #[test]
    fn loopback_is_only_permitted_by_the_test_policy() {
        let strict = Policy::default();
        let test = Policy { allow_loopback: true };
        for ip in ["127.0.0.1", "::1", "::ffff:127.0.0.1"] {
            let ip: IpAddr = ip.parse().unwrap();
            assert!(!strict.permits(ip) && test.permits(ip), "{ip}");
        }
        // The test policy opens loopback, nothing else.
        for ip in ["10.0.0.1", "169.254.169.254", "192.168.0.1", "fd00::1"] {
            assert!(!test.permits(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn urls_are_checked_before_any_request() {
        let check = |url: &str| check_url(&Url::parse(url).unwrap(), Policy::default());
        assert_eq!(check("https://example.com/page"), Ok(()));
        assert_eq!(check("http://example.com:80/"), Ok(()), "the default port is fine");
        assert_eq!(check("ftp://example.com/file"), Err(Refusal::Scheme));
        assert_eq!(check("file:///etc/passwd"), Err(Refusal::Scheme));
        assert_eq!(check("gopher://example.com/"), Err(Refusal::Scheme));
        assert_eq!(check("https://user:pw@example.com/"), Err(Refusal::Credentials));
        assert_eq!(check("http://example.com:8080/"), Err(Refusal::Port));
        assert_eq!(check("http://example.com:22/"), Err(Refusal::Port));
        assert_eq!(check("http://127.0.0.1/"), Err(Refusal::Address("127.0.0.1".parse().unwrap())));
        assert_eq!(check("http://169.254.169.254/latest/meta-data/"), Err(Refusal::Address("169.254.169.254".parse().unwrap())));
        assert_eq!(check("http://[::1]/"), Err(Refusal::Address("::1".parse().unwrap())));
        assert_eq!(check("http://[::ffff:10.0.0.1]/"), Err(Refusal::Address("::ffff:10.0.0.1".parse().unwrap())));
        // Numeric host forms are normalised to an address by the URL parser.
        assert_eq!(check("http://2130706433/"), Err(Refusal::Address("127.0.0.1".parse().unwrap())));
        assert_eq!(check("http://0x7f.1/"), Err(Refusal::Address("127.0.0.1".parse().unwrap())));
        assert_eq!(check("http://017700000001/"), Err(Refusal::Address("127.0.0.1".parse().unwrap())));

        let test = Policy { allow_loopback: true };
        assert_eq!(check_url(&Url::parse("http://127.0.0.1:4567/x").unwrap(), test), Ok(()));
        assert_eq!(check_url(&Url::parse("http://10.0.0.1:4567/x").unwrap(), test), Err(Refusal::Address("10.0.0.1".parse().unwrap())));
    }

    #[tokio::test]
    async fn resolver_refuses_names_that_point_inward() {
        let strict = GuardedResolver { policy: Policy::default() };
        let name = |host: &str| host.parse::<Name>().unwrap();
        // `localhost` resolves to loopback on every system.
        let refused = strict.resolve(name("localhost")).await.err().expect("refused");
        assert!(refused.to_string().contains("not a public address"), "{refused}");

        let test = GuardedResolver { policy: Policy { allow_loopback: true } };
        let addresses: Vec<SocketAddr> = test.resolve(name("localhost")).await.unwrap().collect();
        assert!(!addresses.is_empty() && addresses.iter().all(|a| a.ip().is_loopback()));
    }
}
