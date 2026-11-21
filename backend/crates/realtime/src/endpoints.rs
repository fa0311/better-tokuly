//! `RealtimeService.GetEndpoints`: where and how clients connect.
//!
//! Served by the API process, which need not run a gateway itself: URLs come
//! from configuration and certificate hashes from Valkey, where the gateways
//! publish them.
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::{config::Config, prelude::*};
use tokuly_proto::realtime::{GetEndpointsRequest, GetEndpointsResponse, RealtimeService};

use crate::{Gateway, certs::Certificates, config::RealtimeConfig, ws, wt};

impl RealtimeService for Gateway {
    async fn get_endpoints(&self, _ctx: RequestContext, _request: ServiceRequest<'_, GetEndpointsRequest>) -> ServiceResult<GetEndpointsResponse> {
        handle(async {
            let webtransport_url = webtransport_url(&self.core.cfg, &self.cfg);
            // Hashes exist only for self-signed certificates; with a publicly
            // trusted one (PEM files) the list is empty and clients do not pin.
            let pinned = !webtransport_url.is_empty() && self.cfg.tls_cert.is_none();
            let webtransport_cert_hashes = if pinned { Certificates::published(&self.core.cache).await? } else { Vec::new() };
            ok(GetEndpointsResponse {
                webtransport_url,
                webtransport_cert_hashes,
                websocket_url: websocket_url(&self.core.cfg, &self.cfg),
                ..Default::default()
            })
        })
        .await
    }
}

/// The WebTransport URL for clients; empty when WebTransport is disabled.
/// WebTransport cannot pass through an ordinary HTTP reverse proxy, so by
/// default clients reach the gateway's UDP port on the public host.
pub(crate) fn webtransport_url(core: &Config, cfg: &RealtimeConfig) -> String {
    let Ok(Some(listen)) = cfg.wt_listen_addr() else { return String::new() };
    match configured(&cfg.public_wt_url) {
        Some(url) => url,
        None => format!("https://{}:{}{}", core.public_host(), listen.port(), wt::PATH),
    }
}

/// The WebSocket URL for clients. In production the edge proxy routes
/// `/rt/ws` on the public origin to the gateway; in development clients talk
/// to the gateway's port directly.
pub(crate) fn websocket_url(core: &Config, cfg: &RealtimeConfig) -> String {
    if let Some(url) = configured(&cfg.public_ws_url) {
        return url;
    }
    if core.is_production() {
        let public = core.public_url();
        let origin = match public.strip_prefix("https://") {
            Some(rest) => format!("wss://{rest}"),
            None => format!("ws://{}", public.strip_prefix("http://").unwrap_or(public)),
        };
        format!("{origin}{}", ws::PATH)
    } else {
        format!("ws://{}:{}{}", core.public_host(), cfg.ws_listen.port(), ws::PATH)
    }
}

fn configured(url: &Option<String>) -> Option<String> {
    url.as_deref().map(str::trim).filter(|url| !url.is_empty()).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        core: Config,
    }

    fn core(public_url: &str, env: &str) -> Config {
        Cli::parse_from(["test", "--secret", "0123456789abcdef0123456789abcdef", "--public-url", public_url, "--env", env]).core
    }

    #[test]
    fn development_defaults_point_at_the_gateway_ports() {
        let core = core("http://localhost:8080", "development");
        let cfg = RealtimeConfig::default();
        assert_eq!(webtransport_url(&core, &cfg), "https://localhost:4433/rt");
        assert_eq!(websocket_url(&core, &cfg), "ws://localhost:8082/rt/ws");
    }

    #[test]
    fn production_defaults_use_the_public_origin_for_websocket() {
        let core = core("https://tokuly.example/", "production");
        let cfg = RealtimeConfig { wt_listen: "0.0.0.0:443".into(), ..Default::default() };
        assert_eq!(webtransport_url(&core, &cfg), "https://tokuly.example:443/rt");
        assert_eq!(websocket_url(&core, &cfg), "wss://tokuly.example/rt/ws");

        let plain = self::core("http://tokuly.internal:8080", "production");
        assert_eq!(websocket_url(&plain, &cfg), "ws://tokuly.internal:8080/rt/ws");
    }

    #[test]
    fn configured_urls_win() {
        let core = core("https://tokuly.example", "production");
        let cfg = RealtimeConfig {
            public_wt_url: Some("https://rt.tokuly.example:4433/rt".into()),
            public_ws_url: Some(" wss://rt.tokuly.example/rt/ws ".into()),
            ..Default::default()
        };
        assert_eq!(webtransport_url(&core, &cfg), "https://rt.tokuly.example:4433/rt");
        assert_eq!(websocket_url(&core, &cfg), "wss://rt.tokuly.example/rt/ws");
    }

    #[test]
    fn disabled_webtransport_has_no_url() {
        let core = core("https://tokuly.example", "production");
        let cfg = RealtimeConfig { wt_listen: String::new(), public_wt_url: Some("https://rt.tokuly.example/rt".into()), ..Default::default() };
        assert_eq!(webtransport_url(&core, &cfg), "");
    }
}
