//! SSRF protection for the news fetcher.
//!
//! The fetcher requests URLs that come from outside (an admin's input,
//! `TOKULY_NEWS_FEEDS`, and above all the redirects a feed's server answers
//! with) from inside our network. It must only ever reach the public
//! internet: never loopback, private ranges, link-local addresses (cloud
//! metadata services live there), carrier NAT, multicast, or an IPv6 form
//! that embeds such an IPv4 address.
//!
//! Two checks enforce that, on every redirect hop:
//! - [`check_url`] validates the URL itself (scheme, credentials, port,
//!   literal addresses);
//! - [`GuardedResolver`] validates what a host name *resolves to* at the
//!   moment the connection is made. Checking the name earlier and connecting
//!   later would let a hostile DNS server answer differently the second time
//!   (DNS rebinding).
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use url::{Host, Url};

/// What the fetcher may reach.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Policy {
    /// Also permit loopback addresses and non-default ports: a feed server
    /// on the developer's machine or in a test. Private, link-local and
    /// every other special range stay off limits.
    pub allow_loopback: bool,
}

/// Why a URL, or an address it leads to, was refused.
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
    let special = a == 0                                   // "this network"
        || a == 10                                         // private
        || a == 127                                        // loopback
        || (a == 100 && (64..128).contains(&b))            // carrier-grade NAT
        || (a == 169 && b == 254)                          // link-local, cloud metadata
        || (a == 172 && (16..32).contains(&b))             // private
        || (a == 192 && b == 168)                          // private
        || (a == 192 && b == 0 && c == 0)                  // IETF protocol assignments
        || (a == 192 && b == 0 && c == 2)                  // documentation
        || (a == 192 && b == 88 && c == 99)                // 6to4 relay anycast
        || (a == 198 && (18..20).contains(&b))             // benchmarking
        || (a == 198 && b == 51 && c == 100)               // documentation
        || (a == 203 && b == 0 && c == 113)                // documentation
        || a >= 224; // multicast, reserved, broadcast
    !special
}

/// The IPv4 address carried inside an IPv4-mapped (`::ffff:a.b.c.d`),
/// IPv4-compatible (`::a.b.c.d`), NAT64 (`64:ff9b::/96`) or 6to4
/// (`2002::/16`) IPv6 address.
fn embedded_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let s = ip.segments();
    let low = Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
    match s {
        [0, 0, 0, 0, 0, 0xffff, _, _] => Some(low),
        [0, 0, 0, 0, 0, 0, high, _] if high != 0 => Some(low),
        [0x0064, 0xff9b, 0, 0, 0, 0, _, _] => Some(low),
        [0x2002, high, next, ..] => Some(Ipv4Addr::new((high >> 8) as u8, high as u8, (next >> 8) as u8, next as u8)),
        _ => None,
    }
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    // Forms that carry an IPv4 address are judged by that address.
    if let Some(v4) = embedded_v4(ip) {
        return is_public_v4(v4);
    }
    let s = ip.segments();
    let special = ip.is_unspecified()
        || ip.is_loopback()
        || (s[0] & 0xfe00) == 0xfc00                       // unique local (fc00::/7)
        || (s[0] & 0xffc0) == 0xfe80                       // link-local (fe80::/10)
        || (s[0] & 0xffc0) == 0xfec0                       // site-local (deprecated)
        || (s[0] & 0xff00) == 0xff00                       // multicast
        || (s[0] == 0x2001 && s[1] == 0x0db8)              // documentation
        || (s[0] == 0x2001 && s[1] == 0)                   // Teredo tunnels
        || (s[0] == 0x0100 && s[1..4] == [0, 0, 0]); // discard-only
    !special
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

/// A DNS resolver that only ever yields addresses the policy permits. A
/// name with no permitted address fails to resolve.
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
            "10.0.0.1",
            "10.255.255.255",
            "100.64.0.1",
            "100.127.255.255",
            "127.0.0.1",
            "127.8.9.10",
            "169.254.169.254", // cloud metadata service
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.8",
            "192.0.2.1",
            "192.88.99.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.51.100.7",
            "203.0.113.9",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
        ] {
            assert!(!public(ip), "{ip} must be refused");
        }
        for ip in ["1.1.1.1", "8.8.8.8", "100.63.255.255", "100.128.0.1", "172.15.255.255", "172.32.0.1", "192.167.1.1", "198.20.0.1", "223.255.255.255"] {
            assert!(public(ip), "{ip} is public");
        }
    }

    #[test]
    fn private_and_special_ipv6_ranges_are_not_public() {
        for ip in ["::", "::1", "fc00::1", "fd00:ec2::254", "fe80::1", "febf::1", "fec0::1", "ff02::1", "2001:db8::1", "2001:0:4136:e378:8000:63bf:3fff:fdd2", "100::1"] {
            assert!(!public(ip), "{ip} must be refused");
        }
        for ip in ["2606:4700:4700::1111", "2001:4860:4860::8888", "2a00:1450:4001:81b::200e"] {
            assert!(public(ip), "{ip} is public");
        }
    }

    #[test]
    fn ipv6_forms_embedding_ipv4_are_judged_by_the_ipv4_address() {
        for ip in ["::ffff:127.0.0.1", "::ffff:10.0.0.1", "::ffff:169.254.169.254", "::10.0.0.1", "64:ff9b::192.168.0.1", "2002:7f00:1::1", "2002:a9fe:a9fe::1"] {
            assert!(!public(ip), "{ip} must be refused");
        }
        for ip in ["::ffff:8.8.8.8", "64:ff9b::8.8.8.8", "2002:808:808::1"] {
            assert!(public(ip), "{ip} is public");
        }
    }

    fn check(url: &str, policy: Policy) -> Result<(), Refusal> {
        check_url(&Url::parse(url).unwrap(), policy)
    }

    #[test]
    fn urls_must_be_plain_http_to_a_public_host() {
        let strict = Policy::default();
        assert_eq!(check("https://news.example/rss.xml", strict), Ok(()));
        assert_eq!(check("http://news.example/feed?format=atom", strict), Ok(()));
        assert_eq!(check("http://8.8.8.8/feed", strict), Ok(()));
        assert_eq!(check("ftp://news.example/rss.xml", strict), Err(Refusal::Scheme));
        assert_eq!(check("file:///etc/passwd", strict), Err(Refusal::Scheme));
        assert_eq!(check("https://user:secret@news.example/rss", strict), Err(Refusal::Credentials));
        assert_eq!(check("https://user@news.example/rss", strict), Err(Refusal::Credentials));
        assert_eq!(check("https://news.example:8443/rss", strict), Err(Refusal::Port));
        assert_eq!(check("http://news.example:22/", strict), Err(Refusal::Port));
        // The default port written out is still the default port.
        assert_eq!(check("https://news.example:443/rss", strict), Ok(()));
    }

    #[test]
    fn literal_private_addresses_are_refused_in_every_notation() {
        let strict = Policy::default();
        for url in [
            "http://127.0.0.1/feed",
            "http://localhost/feed", // a name: resolved (and refused) later
            "http://10.1.2.3/feed",
            "http://169.254.169.254/latest/meta-data/",
            "http://192.168.0.10/feed",
            "http://[::1]/feed",
            "http://[::ffff:10.0.0.1]/feed",
            "http://[fd00::1]/feed",
            "http://2130706433/feed", // 127.0.0.1 as one number
            "http://0x7f.0.0.1/feed", // hexadecimal octet
            "http://0177.0.0.1/feed", // octal octet
        ] {
            let result = check(url, strict);
            if url.contains("localhost") {
                assert_eq!(result, Ok(()), "{url}: names are judged when resolved");
            } else {
                assert!(matches!(result, Err(Refusal::Address(_))), "{url} must be refused, got {result:?}");
            }
        }
    }

    #[test]
    fn the_development_policy_adds_loopback_and_ports_only() {
        let dev = Policy { allow_loopback: true };
        assert_eq!(check("http://127.0.0.1:34567/rss.xml", dev), Ok(()));
        assert_eq!(check("http://[::1]:8080/rss.xml", dev), Ok(()));
        assert!(dev.permits("127.0.0.1".parse().unwrap()) && dev.permits("::ffff:127.0.0.1".parse().unwrap()));
        // Everything else that is not public stays refused.
        assert!(matches!(check("http://10.0.0.5:8080/rss.xml", dev), Err(Refusal::Address(_))));
        assert!(matches!(check("http://169.254.169.254/", dev), Err(Refusal::Address(_))));
        assert!(matches!(check("http://192.168.1.1/", dev), Err(Refusal::Address(_))));
        assert_eq!(check("ftp://127.0.0.1/", dev), Err(Refusal::Scheme));
        assert!(!Policy::default().permits("127.0.0.1".parse().unwrap()));
    }

    #[tokio::test]
    async fn the_resolver_refuses_names_that_lead_to_loopback() {
        let name: Name = "localhost".parse().unwrap();
        let strict = GuardedResolver { policy: Policy::default() };
        let error = match strict.resolve(name).await {
            Ok(_) => panic!("localhost must not resolve under the strict policy"),
            Err(error) => error,
        };
        assert!(matches!(error.downcast_ref::<Refusal>(), Some(Refusal::Address(ip)) if ip.is_loopback()), "{error}");

        let name: Name = "localhost".parse().unwrap();
        let dev = GuardedResolver { policy: Policy { allow_loopback: true } };
        let addresses: Vec<SocketAddr> = dev.resolve(name).await.expect("localhost resolves for development").collect();
        assert!(!addresses.is_empty() && addresses.iter().all(|address| address.ip().is_loopback()));
    }
}
