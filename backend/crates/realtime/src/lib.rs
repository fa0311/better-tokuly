//! Realtime gateway: WebTransport + WebSocket fan-out of live events.
//!
//! Clients hold one long-lived connection speaking `tokuly.realtime.v1`
//! frames (see `proto/tokuly/realtime/v1/realtime.proto`). The gateway knows
//! nothing about friends, chats or streams: it authenticates connections,
//! forwards frames the domains publish on the bus (`tokuly_core::realtime`)
//! and hands what clients send to the domains (`tokuly_core::presence`).
//!
//! ```text
//!             ┌─ wt ─┐                    ┌─ hub ──────── bus: rt.u.<user>, rt.t.<topic>
//!   client ───┤      ├─ connection ─ session ─ presence ─ bus: sys.presence · Valkey
//!             └─ ws ─┘      │             ├─ audience ─── Valkey: rt:topic:<topic>
//!                         outbox          └─ (signals) ── bus: loc.sample.<user>, sys.typing, ...
//! ```
//!
//! - `wt` / `ws` turn a transport into a frame source and a frame sink.
//! - `connection` runs one client: a `session` reading frames and a writer
//!   draining the client's bounded `outbox`.
//! - `hub` holds one bus subscription per user/topic and copies frames to
//!   the local connections.
//! - `presence`, `audience` and `certs` keep the shared state in Valkey that
//!   other processes read.
//! - `origin` and `peer` decide who may connect at all.
//!
//! The API process only needs [`Gateway::new`] + [`Gateway::register`]
//! (`GetEndpoints`); [`Gateway::run`] is what makes a process a gateway.
use std::{
    net::SocketAddr,
    ops::Deref,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use tokio::{net::TcpListener, sync::watch, task::JoinHandle};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tokuly_core::{Core, proxies::TrustedProxies};

mod audience;
mod certs;
mod client;
pub mod config;
mod connection;
mod endpoints;
mod frames;
mod hub;
pub mod keys;
mod limits;
mod location;
mod origin;
mod outbox;
mod peer;
mod presence;
mod session;
mod stats;
#[cfg(feature = "testing")]
pub mod testing;
mod topics;
mod ws;
mod wt;

pub use config::RealtimeConfig;
pub use stats::GatewayStats;

use crate::{
    audience::Audience, certs::Certificates, hub::Hub, origin::OriginPolicy, peer::IpLimiter, presence::Presence,
    stats::Counters, wt::WtListener,
};

/// How long the background loops get to withdraw shared state on shutdown.
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(10);

/// Where a running gateway listens (useful with port 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListenAddrs {
    /// TCP address of the WebSocket listener.
    pub websocket: SocketAddr,
    /// UDP address of the WebTransport listener; `None` when disabled.
    pub webtransport: Option<SocketAddr>,
}

pub struct GatewayInner {
    pub core: Core,
    pub cfg: RealtimeConfig,
    /// Distinguishes this process among the gateways (random per start).
    instance: String,
    counters: Arc<Counters>,
    hub: Arc<Hub>,
    presence: Presence,
    audience: Audience,
    certificates: Certificates,
    origins: OriginPolicy,
    trusted_proxies: TrustedProxies,
    ip_limiter: Arc<IpLimiter>,
    /// Periodic guard checks of this gateway, over all connections.
    recheck_limit: parking_lot::Mutex<limits::TokenBucket>,
    /// Cancelled on shutdown: every session says goodbye and ends.
    closing: CancellationToken,
    /// The connection tasks, so shutdown can wait for them.
    connections: TaskTracker,
    listening: watch::Sender<Option<ListenAddrs>>,
    /// Set while the WebTransport listener runs.
    webtransport: watch::Sender<Option<Arc<WtListener>>>,
    started: AtomicBool,
}

/// The realtime gateway. Cheap to clone.
#[derive(Clone)]
pub struct Gateway(Arc<GatewayInner>);

impl Deref for Gateway {
    type Target = GatewayInner;

    fn deref(&self) -> &GatewayInner {
        &self.0
    }
}

impl Gateway {
    /// Prepares a gateway. Nothing listens and nothing runs until
    /// [`Self::run`]; API processes stop here and only [`Self::register`].
    pub fn new(core: Core, cfg: RealtimeConfig) -> anyhow::Result<Self> {
        cfg.validate()?;
        let instance = tokuly_core::ids::random_base62(10);
        let counters = Arc::new(Counters::default());
        let hub = Hub::new(core.bus.clone(), counters.clone());

        let public_wt_url = endpoints::webtransport_url(&core.cfg, &cfg);
        let public_ws_url = endpoints::websocket_url(&core.cfg, &cfg);
        let origins = OriginPolicy::new(
            [core.cfg.public_url(), public_wt_url.as_str(), public_ws_url.as_str()]
                .into_iter()
                .chain(cfg.allowed_origins.iter().map(String::as_str)),
            !core.cfg.is_production(),
        );
        // Self-signed certificates name the public host, although clients
        // pinning the hash do not check it.
        let mut certificate_names = certs::local_names();
        if let Some(host) = url::Url::parse(&public_wt_url)
            .ok()
            .and_then(|url| url.host_str().map(|host| host.trim_matches(['[', ']']).to_owned()))
            && !certificate_names.contains(&host)
        {
            certificate_names.insert(0, host);
        }

        Ok(Self(Arc::new(GatewayInner {
            presence: Presence::new(core.clone(), instance.clone(), cfg.presence_lease, cfg.presence_heartbeat),
            audience: Audience::new(
                core.clone(),
                instance.clone(),
                hub.clone(),
                cfg.topic_stats_interval,
                cfg.audience_refresh,
            ),
            certificates: Certificates::new(core.clone(), cfg.tls_files()?, cfg.cert_validity, certificate_names),
            origins,
            trusted_proxies: cfg.trusted_proxies()?,
            ip_limiter: IpLimiter::new(cfg.max_connections_per_ip),
            recheck_limit: parking_lot::Mutex::new(limits::TokenBucket::new(
                cfg.recheck_rate.ceil().min(f64::from(u32::MAX)) as u32,
                cfg.recheck_rate,
            )),
            closing: CancellationToken::new(),
            connections: TaskTracker::new(),
            listening: watch::Sender::new(None),
            webtransport: watch::Sender::new(None),
            started: AtomicBool::new(false),
            instance,
            counters,
            hub,
            core,
            cfg,
        })))
    }

    /// Registers `RealtimeService` (`GetEndpoints`) on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::realtime::RealtimeServiceExt as _;
        Arc::new(self.clone()).register(router)
    }

    /// Counters of this gateway process.
    pub fn stats(&self) -> GatewayStats {
        self.counters.snapshot()
    }

    /// The name under which this process appears in Valkey and logs.
    pub fn instance_id(&self) -> &str {
        &self.instance
    }

    /// Resolves once [`Self::run`] has bound its listeners.
    pub async fn listening(&self) -> ListenAddrs {
        let mut listening = self.listening.subscribe();
        loop {
            if let Some(addrs) = *listening.borrow_and_update() {
                return addrs;
            }
            if listening.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    }

    /// Announces the next self-signed certificate ahead of schedule:
    /// `GetEndpoints` returns its hash next to the current one from now on.
    /// For operations and tests; the gateway does this by itself at 60% of
    /// the certificate lifetime. No-op with PEM files.
    pub async fn announce_next_certificate(&self) -> anyhow::Result<()> {
        self.certificates.announce_next().await
    }

    /// Switches the WebTransport listener to the announced certificate (done
    /// automatically at 75% of the lifetime). No-op when nothing was
    /// announced or WebTransport is not running.
    pub async fn activate_next_certificate(&self) -> anyhow::Result<()> {
        let listener = self.webtransport.borrow().clone();
        match listener {
            Some(listener) => self.certificates.activate_next(&listener).await,
            None => Ok(()),
        }
    }

    /// Serves clients until `core.shutdown` is cancelled, then closes the
    /// connections gracefully and withdraws this gateway's shared state.
    pub async fn run(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.started.swap(true, Ordering::SeqCst), "Gateway::run was already called");
        let cfg = &self.cfg;

        let ws_listener = TcpListener::bind(cfg.ws_listen)
            .await
            .map_err(|e| anyhow::anyhow!("cannot listen for WebSocket on tcp/{}: {e}", cfg.ws_listen))?;
        let ws_addr = ws_listener.local_addr()?;
        let wt_listener = match cfg.wt_listen_addr()? {
            Some(addr) => {
                let identity = self.certificates.load().await?;
                let listener = Arc::new(WtListener::bind(addr, identity, cfg)?);
                self.certificates.publish().await?;
                Some(listener)
            }
            None => None,
        };
        let addrs = ListenAddrs {
            websocket: ws_addr,
            webtransport: wt_listener.as_ref().map(|listener| listener.local_addr()),
        };
        tracing::info!(
            instance = self.instance,
            websocket = %addrs.websocket,
            webtransport = addrs.webtransport.map(|addr| addr.to_string()),
            self_signed = cfg.tls_cert.is_none(),
            "realtime gateway listening"
        );

        // Accept loops stop first, the helpers last: they still have to
        // record the departure of every connection.
        let accepting = CancellationToken::new();
        let helpers = CancellationToken::new();
        let mut ws_task = tokio::spawn(ws::serve(self.clone(), ws_listener, accepting.clone()));
        let wt_task = wt_listener.clone().map(|listener| {
            let (gateway, accepting) = (self.clone(), accepting.clone());
            tokio::spawn(async move { listener.serve(gateway, accepting).await })
        });
        let helper_tasks = self.spawn_helpers(wt_listener.clone(), &helpers);
        self.webtransport.send_replace(wt_listener.clone());
        self.listening.send_replace(Some(addrs));

        let ws_failure = tokio::select! {
            _ = self.core.shutdown.cancelled() => None,
            ended = &mut ws_task => Some(match ended {
                Ok(Ok(())) => anyhow::anyhow!("WebSocket listener stopped unexpectedly"),
                Ok(Err(error)) => anyhow::anyhow!("WebSocket listener failed: {error}"),
                Err(error) => anyhow::anyhow!("WebSocket listener panicked: {error}"),
            }),
        };

        tracing::info!(
            instance = self.instance,
            connections = self.counters.open_connections(),
            "realtime gateway shutting down"
        );
        accepting.cancel();
        self.closing.cancel();
        self.connections.close();
        if tokio::time::timeout(cfg.shutdown_grace, self.connections.wait()).await.is_err() {
            tracing::warn!(
                connections = self.counters.open_connections(),
                "connections still open after the shutdown grace period"
            );
        }
        if let Some(listener) = &wt_listener {
            listener.close(cfg.shutdown_grace).await;
        }
        self.webtransport.send_replace(None);
        if let Some(task) = wt_task {
            let _ = task.await;
        }
        if ws_failure.is_none() {
            let _ = ws_task.await;
        }

        helpers.cancel();
        let cleanup = async {
            for task in helper_tasks {
                let _ = task.await;
            }
        };
        if tokio::time::timeout(CLEANUP_TIMEOUT, cleanup).await.is_err() {
            tracing::warn!("realtime gateway cleanup timed out; leases and counts will expire on their own");
        }
        self.hub.shutdown();
        match ws_failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Starts the loops that maintain shared state while the gateway runs.
    /// Each cleans up after itself when `stop` is cancelled.
    fn spawn_helpers(&self, wt_listener: Option<Arc<WtListener>>, stop: &CancellationToken) -> Vec<JoinHandle<()>> {
        let mut tasks = Vec::new();
        let (gateway, token) = (self.clone(), stop.clone());
        tasks.push(tokio::spawn(async move { gateway.presence.run(token).await }));
        let (gateway, token) = (self.clone(), stop.clone());
        tasks.push(tokio::spawn(async move { gateway.audience.run(token).await }));
        let (gateway, token) = (self.clone(), stop.clone());
        tasks.push(tokio::spawn(async move {
            gateway.counters.log_periodically(&gateway.instance, gateway.cfg.stats_interval, token).await
        }));
        if let Some(listener) = wt_listener {
            let (gateway, token) = (self.clone(), stop.clone());
            tasks.push(tokio::spawn(async move { gateway.certificates.run(&listener, token).await }));
        }
        tasks
    }

    pub(crate) fn inner(&self) -> Arc<GatewayInner> {
        self.0.clone()
    }
}

impl GatewayInner {
    /// Whether the connection limit is reached.
    pub(crate) fn at_capacity(&self) -> bool {
        self.counters.open_connections() >= self.cfg.max_connections as u64
    }
}
