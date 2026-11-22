//! Live (ライブ配信): live streams, ingest through MediaMTX, live chat,
//! gifts, polls, clips, OBS overlays and analytics for the streamer.
//!
//! # For other domains and the server
//!
//! - [`Live::new`] takes the wallet (gifts move coins); [`Live::register`]
//!   adds the seven Connect services; [`Live::http_routes`] is the hook the
//!   media server calls (mount at `/hooks/live`, not on the public edge);
//!   [`Live::spawn_workers`] starts the reconciler, the jobs and the event
//!   consumer in worker processes.
//! - [`clips`]: [`clips::find`] and [`clips::ClipRow`] for the shorts domain
//!   ("make a short from a clip").
//! - [`streams`]: [`streams::find`], [`streams::StreamRow`] and
//!   [`streams::current_of_channel`] (is this channel on air?).
//! - [`events`]: topics and payloads of what this domain emits.
//!
//! # Map of the crate
//!
//! | module | what lives there |
//! |---|---|
//! | [`access`] | who may see a stream, as code and as SQL predicates; chat standing |
//! | [`streams`] | stream rows, lookups, create / update / delete, stream keys |
//! | [`lifecycle`] | going on air, ending, following the media server, viewer samples, the media timeline |
//! | [`media_server`], [`hooks`] | MediaMTX: URLs handed to clients, its control API, the auth hook it calls |
//! | [`audience`] | who is watching (Valkey) |
//! | [`chat`], [`polls`], [`gifts`], [`tiers`] | live chat and its moderation, polls, gifts and the gift ladder |
//! | [`recordings`], [`clips`], [`ffmpeg`] | recordings on disk, the archive job, clips |
//! | [`overlays`] | OBS widgets, their presets and tokens, topic guards |
//! | [`analytics`] | numbers for the streamer |
//! | [`announce`] | telling subscribers, friends and reminded people that a stream started |
//! | `*_svc` | the Connect services: handlers over the modules above |
//! | [`workers`] | event consumers, jobs and timers |
//! | [`subjects`] | subject resolvers (likes, reports) and data export |
//! | [`events`] | topics, payloads and job names |
use std::{ops::Deref, sync::Arc};

use tokuly_core::Core;
use tokuly_identity::Identity;
use tokuly_wallet::Wallet;

pub mod access;
pub mod analytics;
pub mod announce;
pub mod audience;
pub mod chat;
pub mod clips;
pub mod config;
pub mod events;
pub mod ffmpeg;
pub mod gifts;
pub mod hooks;
pub mod lifecycle;
pub mod media_server;
pub mod overlays;
pub mod polls;
pub mod recordings;
pub mod search;
pub mod streams;
pub mod subjects;
pub mod tiers;
pub mod workers;

mod analytics_svc;
mod chat_svc;
mod clip_svc;
mod gift_svc;
mod overlay_svc;
mod poll_svc;
mod push;
mod stream_svc;

pub use config::LiveConfig;

pub struct LiveInner {
    pub core: Core,
    pub identity: Identity,
    pub cfg: LiveConfig,
    /// Gifts move coins through the wallet.
    pub wallet: Wallet,
    /// The widgets of the channels that were recently pushed to.
    pub(crate) overlay_topics: overlays::TopicCache,
}

/// The live domain. Cheap to clone.
#[derive(Clone)]
pub struct Live(Arc<LiveInner>);

impl Deref for Live {
    type Target = LiveInner;

    fn deref(&self) -> &LiveInner {
        &self.0
    }
}

impl Live {
    /// Builds the domain and plugs it into the shared registries: subject
    /// resolvers for streams, clips and chat messages, the `live` section of
    /// the personal data export, and the guards of the `live:` and
    /// `overlay:` realtime topics (which must be in place in every process,
    /// the gateway's included).
    pub fn new(core: Core, identity: Identity, cfg: LiveConfig, wallet: Wallet) -> anyhow::Result<Self> {
        cfg.validate()?;
        subjects::register(&core);
        overlays::register_guards(&core);
        if !cfg.media_server_configured() {
            tracing::info!(
                "no media server configured (TOKULY_MEDIAMTX_API_URL): streams can be managed but not go on air"
            );
        }
        Ok(Self(Arc::new(LiveInner { core, identity, cfg, wallet, overlay_topics: Default::default() })))
    }

    /// Registers the live services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::live::{
            ClipServiceExt, LiveAnalyticsServiceExt, LiveChatServiceExt, LiveGiftServiceExt, LivePollServiceExt,
            LiveStreamServiceExt, OverlayServiceExt,
        };
        let me = Arc::new(self.clone());
        let router = LiveStreamServiceExt::register(me.clone(), router);
        let router = LiveChatServiceExt::register(me.clone(), router);
        let router = LivePollServiceExt::register(me.clone(), router);
        let router = LiveGiftServiceExt::register(me.clone(), router);
        let router = ClipServiceExt::register(me.clone(), router);
        let router = OverlayServiceExt::register(me.clone(), router);
        LiveAnalyticsServiceExt::register(me, router)
    }
}
