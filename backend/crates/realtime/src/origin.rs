//! Which browser origins may open a realtime connection.
//!
//! Connections authenticate with a token inside the `Hello` frame, never
//! with cookies, so a foreign page cannot act as a signed-in user. The
//! origin check is still worth having: it keeps other sites from using the
//! gateway as their own anonymous event stream.
use url::Url;

/// Schemes that browsers, WebSocket clients and the proxy may use for what
/// is one and the same origin.
fn web_scheme(scheme: &str) -> &str {
    match scheme {
        "ws" => "http",
        "wss" => "https",
        other => other,
    }
}

/// `scheme://host[:port]` of a URL, in the form browsers send as `Origin`.
fn origin_of(url: &str) -> Option<String> {
    let url = Url::parse(url.trim()).ok()?;
    let host = url.host_str()?;
    let scheme = web_scheme(url.scheme());
    Some(match url.port() {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    })
}

pub(crate) struct OriginPolicy {
    /// Normalized origins that are always allowed.
    allowed: Vec<String>,
    /// Development: any page served from this machine may connect.
    allow_local: bool,
}

impl OriginPolicy {
    /// `urls` are the public URL of the app, the public gateway URLs and the
    /// operator's extra origins; entries that are not URLs are ignored.
    pub(crate) fn new<'a>(urls: impl IntoIterator<Item = &'a str>, allow_local: bool) -> Self {
        let mut allowed: Vec<String> = urls.into_iter().filter_map(origin_of).collect();
        allowed.sort();
        allowed.dedup();
        Self { allowed, allow_local }
    }

    /// Whether a connection request may proceed. `origin` is the `Origin`
    /// header; `host` is the authority the request was sent to.
    pub(crate) fn allows(&self, origin: Option<&str>, host: Option<&str>) -> bool {
        // Native apps and server-side clients send no Origin: nothing to check.
        let Some(origin) = origin else { return true };
        let Some(normalized) = origin_of(origin) else { return false };
        if self.allowed.contains(&normalized) {
            return true;
        }
        // Some native WebSocket stacks (React Native) invent an Origin from
        // the URL they connect to. No web page lives on the gateway's own
        // address, so an origin equal to it cannot be a foreign site.
        if let (Some(host), Ok(url)) = (host, Url::parse(&normalized))
            && authority_of(&url).eq_ignore_ascii_case(host.trim())
        {
            return true;
        }
        self.allow_local && is_local(&normalized)
    }
}

fn authority_of(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    }
}

/// Dev servers, Expo web and Tauri run on assorted local ports.
fn is_local(origin: &str) -> bool {
    let Ok(url) = Url::parse(origin) else { return false };
    match (url.scheme(), url.host_str()) {
        ("http" | "https", Some(host)) => matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "tauri.localhost"),
        ("tauri", Some("localhost")) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn production() -> OriginPolicy {
        OriginPolicy::new(["https://tokuly.example", "wss://rt.tokuly.example/rt/ws", "https://studio.tokuly.example/", "", "not a url"], false)
    }

    #[test]
    fn configured_origins_are_allowed() {
        let policy = production();
        assert!(policy.allows(Some("https://tokuly.example"), None));
        assert!(policy.allows(Some("https://studio.tokuly.example"), None));
        // The gateway's own public URL counts, with the scheme a browser would send.
        assert!(policy.allows(Some("https://rt.tokuly.example"), None));
        assert!(!policy.allows(Some("https://evil.example"), None));
        assert!(!policy.allows(Some("http://tokuly.example"), None), "scheme matters");
        assert!(!policy.allows(Some("https://tokuly.example:8443"), None), "port matters");
        assert!(!policy.allows(Some("null"), None));
    }

    #[test]
    fn clients_without_origin_are_allowed() {
        assert!(production().allows(None, None));
    }

    #[test]
    fn an_origin_equal_to_the_gateway_address_is_allowed() {
        let policy = production();
        assert!(policy.allows(Some("http://192.168.1.20:8082"), Some("192.168.1.20:8082")));
        assert!(!policy.allows(Some("http://192.168.1.20:3000"), Some("192.168.1.20:8082")));
    }

    #[test]
    fn localhost_is_allowed_in_development_only() {
        let development = OriginPolicy::new(["http://localhost:8080"], true);
        for origin in ["http://localhost:5173", "http://127.0.0.1:3000", "https://localhost", "tauri://localhost", "http://tauri.localhost", "http://[::1]:19006"] {
            assert!(development.allows(Some(origin), None), "{origin}");
            assert!(!production().allows(Some(origin), None), "{origin}");
        }
        assert!(!development.allows(Some("http://localhost.evil.example"), None));
        assert!(!development.allows(Some("https://evil.example"), None));
    }
}
