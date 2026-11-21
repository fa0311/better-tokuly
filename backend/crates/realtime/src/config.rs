//! Gateway settings.
//!
//! Durations are written as a number with a unit (`500ms`, `25s`, `2m`, `1h`,
//! `13d`); a bare number means seconds.
use std::{net::SocketAddr, path::PathBuf, time::Duration};

use clap::Args;
use tokuly_core::{
    presence::{AUDIENCE_TTL_SECONDS, ONLINE_TTL_SECONDS},
    proxies::{DEFAULT_TRUSTED_PROXIES, TrustedProxies},
};

/// Browsers refuse `serverCertificateHashes` certificates valid for more than
/// 14 days; we stay a day under that.
pub const MAX_SELF_SIGNED_VALIDITY: Duration = Duration::from_secs(13 * 24 * 3600);

#[derive(Debug, Clone, Args)]
pub struct RealtimeConfig {
    // ── Listeners ──
    /// UDP address of the WebTransport (HTTP/3) listener. Empty disables WebTransport.
    #[arg(long = "rt-wt-listen", env = "TOKULY_RT_WT_LISTEN", default_value = "0.0.0.0:4433")]
    pub wt_listen: String,
    /// TCP address of the WebSocket fallback listener (`/rt/ws`, `/healthz`).
    #[arg(long = "rt-ws-listen", env = "TOKULY_RT_WS_LISTEN", default_value = "0.0.0.0:8082")]
    pub ws_listen: SocketAddr,

    // ── TLS identity of the WebTransport listener ──
    /// PEM certificate chain. With `TOKULY_RT_TLS_KEY` it replaces the
    /// self-signed certificate; use a publicly trusted one.
    #[arg(long = "rt-tls-cert", env = "TOKULY_RT_TLS_CERT")]
    pub tls_cert: Option<PathBuf>,
    /// PEM private key belonging to `TOKULY_RT_TLS_CERT`.
    #[arg(long = "rt-tls-key", env = "TOKULY_RT_TLS_KEY")]
    pub tls_key: Option<PathBuf>,
    /// Lifetime of each self-signed certificate (at most 13 days). It is
    /// announced at 60% and replaced at 75% of this time.
    #[arg(long = "rt-cert-validity", env = "TOKULY_RT_CERT_VALIDITY", default_value = "13d", value_parser = parse_duration)]
    pub cert_validity: Duration,

    // ── What clients are told (GetEndpoints) ──
    /// WebTransport URL given to clients. Default: `https://<public host>:<WebTransport port>/rt`.
    #[arg(long = "rt-public-wt-url", env = "TOKULY_RT_PUBLIC_WT_URL")]
    pub public_wt_url: Option<String>,
    /// WebSocket URL given to clients. Default: `wss://<public origin>/rt/ws`
    /// in production (through the edge proxy), `ws://<public host>:<WebSocket
    /// port>/rt/ws` in development.
    #[arg(long = "rt-public-ws-url", env = "TOKULY_RT_PUBLIC_WS_URL")]
    pub public_ws_url: Option<String>,
    /// Extra browser origins allowed to connect (comma separated). The public
    /// URL is always allowed; in development so is any localhost port.
    #[arg(long = "rt-allowed-origins", env = "TOKULY_RT_ALLOWED_ORIGINS", value_delimiter = ',', default_value = "")]
    pub allowed_origins: Vec<String>,

    // ── Admission ──
    /// Connections accepted by one gateway process before new ones are refused.
    #[arg(long = "rt-max-connections", env = "TOKULY_RT_MAX_CONNECTIONS", default_value_t = 100_000)]
    pub max_connections: usize,
    /// Connections one client address (an IPv4 address or an IPv6 /64) may
    /// hold at a time, over both transports; 0 disables the limit. The
    /// default leaves room for a campus or a mobile carrier behind one
    /// address while keeping any single one far from filling the gateway.
    #[arg(long = "rt-max-connections-per-ip", env = "TOKULY_RT_MAX_CONNECTIONS_PER_IP", default_value_t = 1024)]
    pub max_connections_per_ip: usize,
    /// Proxies whose `X-Forwarded-For` header names the real client of a
    /// WebSocket connection (addresses or CIDR ranges, comma separated).
    /// From any other peer the header is ignored. Add the ranges of a CDN in
    /// front of the edge proxy here; empty if the gateway is exposed directly.
    #[arg(id = "rt_trusted_proxies", long = "rt-trusted-proxies", env = "TOKULY_RT_TRUSTED_PROXIES", value_delimiter = ',', default_value = DEFAULT_TRUSTED_PROXIES)]
    pub trusted_proxies: Vec<String>,

    // ── Connection lifecycle ──
    /// How long a new connection may take to send its `Hello`.
    #[arg(long = "rt-hello-timeout", env = "TOKULY_RT_HELLO_TIMEOUT", default_value = "10s", value_parser = parse_duration)]
    pub hello_timeout: Duration,
    /// Ping cadence asked of clients in `Welcome`.
    #[arg(long = "rt-ping-interval", env = "TOKULY_RT_PING_INTERVAL", default_value = "25s", value_parser = parse_duration)]
    pub ping_interval: Duration,
    /// A connection that sent nothing for this long is dropped.
    #[arg(long = "rt-idle-timeout", env = "TOKULY_RT_IDLE_TIMEOUT", default_value = "60s", value_parser = parse_duration)]
    pub idle_timeout: Duration,
    /// After its access token expires a connection has this long to send a
    /// `Hello` with a fresh one.
    #[arg(long = "rt-auth-grace", env = "TOKULY_RT_AUTH_GRACE", default_value = "60s", value_parser = parse_duration)]
    pub auth_grace: Duration,
    /// How often each connection re-checks that its session was not revoked.
    #[arg(long = "rt-revocation-check", env = "TOKULY_RT_REVOCATION_CHECK", default_value = "10s", value_parser = parse_duration)]
    pub revocation_check: Duration,
    /// How long shutdown waits for connections to receive their goodbye.
    #[arg(long = "rt-shutdown-grace", env = "TOKULY_RT_SHUTDOWN_GRACE", default_value = "5s", value_parser = parse_duration)]
    pub shutdown_grace: Duration,

    // ── Per-connection limits ──
    /// Largest accepted client frame, in bytes.
    #[arg(long = "rt-max-frame-bytes", env = "TOKULY_RT_MAX_FRAME_BYTES", default_value_t = 64 * 1024)]
    pub max_frame_bytes: usize,
    /// Topics one connection may be subscribed to at a time.
    #[arg(long = "rt-max-topics", env = "TOKULY_RT_MAX_TOPICS", default_value_t = 64)]
    pub max_topics: usize,
    /// Reliable frames queued for a slow client before it is disconnected.
    #[arg(long = "rt-queue-frames", env = "TOKULY_RT_QUEUE_FRAMES", default_value_t = 1024)]
    pub queue_frames: usize,
    /// Bytes of reliable frames queued for a slow client before it is disconnected.
    #[arg(long = "rt-queue-bytes", env = "TOKULY_RT_QUEUE_BYTES", default_value_t = 4 * 1024 * 1024)]
    pub queue_bytes: usize,
    /// Droppable frames (friend locations) queued per client; beyond this the
    /// oldest is discarded.
    #[arg(long = "rt-queue-droppable-frames", env = "TOKULY_RT_QUEUE_DROPPABLE_FRAMES", default_value_t = 128)]
    pub queue_droppable_frames: usize,
    /// A single write that makes no progress for this long closes the connection.
    #[arg(long = "rt-write-timeout", env = "TOKULY_RT_WRITE_TIMEOUT", default_value = "15s", value_parser = parse_duration)]
    pub write_timeout: Duration,
    /// Location samples one connection may send in a burst...
    #[arg(long = "rt-location-burst", env = "TOKULY_RT_LOCATION_BURST", default_value_t = 60)]
    pub location_burst: u32,
    /// ...and per second once the burst is used up.
    #[arg(long = "rt-location-rate", env = "TOKULY_RT_LOCATION_RATE", default_value_t = 2.0)]
    pub location_rate: f64,

    // ── Hints passed on in `Welcome` ──
    /// Suggested location report cadence while moving.
    #[arg(long = "rt-moving-interval", env = "TOKULY_RT_MOVING_INTERVAL", default_value = "5s", value_parser = parse_duration)]
    pub moving_interval: Duration,
    /// Suggested location report cadence while stationary.
    #[arg(long = "rt-stationary-interval", env = "TOKULY_RT_STATIONARY_INTERVAL", default_value = "60s", value_parser = parse_duration)]
    pub stationary_interval: Duration,

    // ── Presence and audiences ──
    /// How often online markers of connected users are refreshed.
    #[arg(long = "rt-presence-heartbeat", env = "TOKULY_RT_PRESENCE_HEARTBEAT", default_value = "25s", value_parser = parse_duration)]
    pub presence_heartbeat: Duration,
    /// How long a user stays online after their gateway stopped refreshing
    /// (i.e. crashed). Must exceed the heartbeat.
    #[arg(long = "rt-presence-lease", env = "TOKULY_RT_PRESENCE_LEASE", default_value = "75s", value_parser = parse_duration)]
    pub presence_lease: Duration,
    /// How often topic audience reports are rewritten (they expire after 90 s; at most 45s).
    #[arg(long = "rt-audience-refresh", env = "TOKULY_RT_AUDIENCE_REFRESH", default_value = "30s", value_parser = parse_duration)]
    pub audience_refresh: Duration,
    /// Minimum time between two `TopicStats` pushes for one topic.
    #[arg(long = "rt-topic-stats-interval", env = "TOKULY_RT_TOPIC_STATS_INTERVAL", default_value = "2s", value_parser = parse_duration)]
    pub topic_stats_interval: Duration,

    // ── Observability ──
    /// How often connection and frame counters are logged.
    #[arg(long = "rt-stats-interval", env = "TOKULY_RT_STATS_INTERVAL", default_value = "60s", value_parser = parse_duration)]
    pub stats_interval: Duration,
}

impl Default for RealtimeConfig {
    fn default() -> Self {
        Self {
            wt_listen: "0.0.0.0:4433".to_owned(),
            ws_listen: SocketAddr::from(([0, 0, 0, 0], 8082)),
            tls_cert: None,
            tls_key: None,
            cert_validity: MAX_SELF_SIGNED_VALIDITY,
            public_wt_url: None,
            public_ws_url: None,
            allowed_origins: Vec::new(),
            max_connections: 100_000,
            max_connections_per_ip: 1024,
            trusted_proxies: DEFAULT_TRUSTED_PROXIES.split(',').map(str::to_owned).collect(),
            hello_timeout: Duration::from_secs(10),
            ping_interval: Duration::from_secs(25),
            idle_timeout: Duration::from_secs(60),
            auth_grace: Duration::from_secs(60),
            revocation_check: Duration::from_secs(10),
            shutdown_grace: Duration::from_secs(5),
            max_frame_bytes: 64 * 1024,
            max_topics: 64,
            queue_frames: 1024,
            queue_bytes: 4 * 1024 * 1024,
            queue_droppable_frames: 128,
            write_timeout: Duration::from_secs(15),
            location_burst: 60,
            location_rate: 2.0,
            moving_interval: Duration::from_secs(5),
            stationary_interval: Duration::from_secs(60),
            presence_heartbeat: Duration::from_secs(25),
            presence_lease: Duration::from_secs(ONLINE_TTL_SECONDS),
            audience_refresh: Duration::from_secs(30),
            topic_stats_interval: Duration::from_secs(2),
            stats_interval: Duration::from_secs(60),
        }
    }
}

impl RealtimeConfig {
    /// The WebTransport listen address; `None` when WebTransport is disabled.
    pub fn wt_listen_addr(&self) -> anyhow::Result<Option<SocketAddr>> {
        let value = self.wt_listen.trim();
        if value.is_empty() {
            return Ok(None);
        }
        value
            .parse()
            .map(Some)
            .map_err(|e| anyhow::anyhow!("TOKULY_RT_WT_LISTEN `{value}` is not a socket address: {e}"))
    }

    /// PEM files of an operator-provided certificate, when both are set.
    pub fn tls_files(&self) -> anyhow::Result<Option<(PathBuf, PathBuf)>> {
        match (&self.tls_cert, &self.tls_key) {
            (Some(cert), Some(key)) => Ok(Some((cert.clone(), key.clone()))),
            (None, None) => Ok(None),
            _ => anyhow::bail!("TOKULY_RT_TLS_CERT and TOKULY_RT_TLS_KEY must be set together"),
        }
    }

    /// The parsed `TOKULY_RT_TRUSTED_PROXIES`.
    pub(crate) fn trusted_proxies(&self) -> anyhow::Result<TrustedProxies> {
        TrustedProxies::parse(&self.trusted_proxies).map_err(|e| anyhow::anyhow!("TOKULY_RT_TRUSTED_PROXIES: {e}"))
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        self.wt_listen_addr()?;
        self.tls_files()?;
        self.trusted_proxies()?;
        anyhow::ensure!(
            !self.cert_validity.is_zero() && self.cert_validity <= MAX_SELF_SIGNED_VALIDITY,
            "TOKULY_RT_CERT_VALIDITY must be between 1s and 13d (browsers reject longer-lived certificates for serverCertificateHashes)"
        );
        anyhow::ensure!(
            self.idle_timeout > self.ping_interval,
            "TOKULY_RT_IDLE_TIMEOUT must be longer than TOKULY_RT_PING_INTERVAL"
        );
        anyhow::ensure!(
            self.presence_lease > self.presence_heartbeat,
            "TOKULY_RT_PRESENCE_LEASE must be longer than TOKULY_RT_PRESENCE_HEARTBEAT"
        );
        anyhow::ensure!(
            self.audience_refresh.as_secs_f64() * 2.0 <= AUDIENCE_TTL_SECONDS as f64,
            "TOKULY_RT_AUDIENCE_REFRESH must be at most {}s: audience reports expire after {AUDIENCE_TTL_SECONDS}s and should survive one missed refresh",
            AUDIENCE_TTL_SECONDS / 2
        );
        anyhow::ensure!(
            self.location_rate.is_finite() && self.location_rate > 0.0 && self.location_burst > 0,
            "TOKULY_RT_LOCATION_RATE and TOKULY_RT_LOCATION_BURST must be positive"
        );
        anyhow::ensure!(
            self.max_topics > 0 && self.queue_frames > 0 && self.queue_bytes > 0 && self.queue_droppable_frames > 0 && self.max_frame_bytes >= 1024,
            "realtime queue, topic and frame limits must be positive (frames at least 1024 bytes)"
        );
        for (name, value) in [
            ("TOKULY_RT_HELLO_TIMEOUT", self.hello_timeout),
            ("TOKULY_RT_PING_INTERVAL", self.ping_interval),
            ("TOKULY_RT_REVOCATION_CHECK", self.revocation_check),
            ("TOKULY_RT_WRITE_TIMEOUT", self.write_timeout),
            ("TOKULY_RT_PRESENCE_HEARTBEAT", self.presence_heartbeat),
            ("TOKULY_RT_AUDIENCE_REFRESH", self.audience_refresh),
            ("TOKULY_RT_TOPIC_STATS_INTERVAL", self.topic_stats_interval),
            ("TOKULY_RT_STATS_INTERVAL", self.stats_interval),
        ] {
            anyhow::ensure!(!value.is_zero(), "{name} must be greater than zero");
        }
        Ok(())
    }
}

/// Parses `500ms`, `25s`, `2m`, `1h`, `13d` or a bare number of seconds.
pub fn parse_duration(text: &str) -> Result<Duration, String> {
    let text = text.trim();
    let split = text.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let value: f64 = number.parse().map_err(|_| format!("`{text}` is not a duration (examples: 500ms, 25s, 2m, 1h, 13d)"))?;
    let seconds = match unit.trim() {
        "ms" => value / 1000.0,
        "" | "s" => value,
        "m" => value * 60.0,
        "h" => value * 3600.0,
        "d" => value * 86_400.0,
        other => return Err(format!("unknown duration unit `{other}` (use ms, s, m, h or d)")),
    };
    Duration::try_from_secs_f64(seconds).map_err(|e| format!("`{text}` is not a usable duration: {e}"))
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        rt: RealtimeConfig,
    }

    #[test]
    fn durations_accept_units_and_bare_seconds() {
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_duration("25").unwrap(), Duration::from_secs(25));
        assert_eq!(parse_duration("25s").unwrap(), Duration::from_secs(25));
        assert_eq!(parse_duration("1.5m").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
        assert_eq!(parse_duration("13d").unwrap(), MAX_SELF_SIGNED_VALIDITY);
        assert!(parse_duration("soon").is_err());
        assert!(parse_duration("5 weeks").is_err());
        assert!(parse_duration("").is_err());
    }

    #[test]
    fn command_line_defaults_match_the_default_impl() {
        let mut parsed = Cli::parse_from(["test"]).rt;
        // An empty list arrives from the command line as one empty entry.
        parsed.allowed_origins.retain(|origin| !origin.is_empty());
        let default = RealtimeConfig::default();
        // `Debug` covers every field, so a default changed in only one place fails here.
        assert_eq!(format!("{parsed:?}"), format!("{default:?}"));
        default.validate().unwrap();
    }

    #[test]
    fn validation_rejects_inconsistent_settings() {
        let invalid = |change: fn(&mut RealtimeConfig)| {
            let mut cfg = RealtimeConfig::default();
            change(&mut cfg);
            cfg.validate().is_err()
        };
        assert!(invalid(|c| c.wt_listen = "nonsense".into()));
        assert!(invalid(|c| c.tls_cert = Some("cert.pem".into())));
        assert!(invalid(|c| c.cert_validity = Duration::from_secs(14 * 24 * 3600)));
        assert!(invalid(|c| c.idle_timeout = c.ping_interval));
        assert!(invalid(|c| c.presence_lease = c.presence_heartbeat));
        assert!(invalid(|c| c.location_rate = 0.0));
        assert!(invalid(|c| c.max_topics = 0));
        assert!(invalid(|c| c.audience_refresh = Duration::from_secs(60)));
        assert!(invalid(|c| c.trusted_proxies = vec!["10.0.0.0/40".into()]));

        let disabled = RealtimeConfig { wt_listen: String::new(), ..Default::default() };
        assert_eq!(disabled.wt_listen_addr().unwrap(), None);
    }
}
