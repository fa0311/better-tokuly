//! Video (動画): channels, long-form videos, subtitles, playlists, watch
//! history, feeds, watch parties and creator analytics.
//!
//! # For other domains
//!
//! The live and shorts domains are built on this crate's channels:
//!
//! - [`channels`]: [`channels::summaries`], [`channels::require_owned`],
//!   [`channels::default_for`], [`channels::subscriber_ids`],
//!   [`channels::is_subscribed`], [`channels::ChannelRow`], [`channels::NotifyLevel`];
//! - [`videos`]: [`videos::find`], [`videos::VideoRow`] and
//!   [`videos::create_from_asset`] (a live archive becomes a video);
//! - [`access::Viewer`] for the visibility rules (who may watch what);
//! - [`games::record_use`] to feed the game title suggestions;
//! - [`party::PartyStamps`] / [`Video::set_party_stamps`]: how the server
//!   lends the stamp domain to party chat.
//!
//! # Map of the crate
//!
//! | module | what lives there |
//! |---|---|
//! | [`access`] | every rule about who may see what, as code and as SQL predicates |
//! | [`validate`] | validation of names, titles, tags, chapters, links, language tags |
//! | [`channels`], [`videos`], [`playlists`], [`subtitles`], [`party`] | rows, lookups and the changes to each entity |
//! | [`playback`] | playback sessions, view counting, watch history |
//! | [`feeds`] | home, subscriptions, trending, related; the trending score |
//! | [`analytics`] | creator analytics from ClickHouse |
//! | [`games`], [`search`] | game title suggestions; search documents |
//! | `*_svc` | the seven Connect services: handlers over the modules above |
//! | [`workers`] | event consumers, the announcement job and the timers |
//! | [`subjects`] | subject resolvers (comments, reactions, reports) and data export |
//! | [`events`] | topics and payloads of the events emitted and consumed |
use std::{ops::Deref, sync::Arc};

use parking_lot::RwLock;
use tokuly_core::{
    Core,
    prelude::{AppResult, Uuid},
};
use tokuly_identity::Identity;

pub mod access;
pub mod analytics;
pub mod channels;
pub mod config;
pub mod events;
pub mod feeds;
pub mod games;
pub mod locks;
pub mod party;
pub mod playback;
pub mod playlists;
pub mod search;
pub mod subjects;
pub mod subtitles;
pub mod validate;
pub mod videos;
pub mod workers;

mod analytics_svc;
mod channel_svc;
mod feed_svc;
mod party_svc;
mod playback_svc;
mod playlist_svc;
mod video_svc;

pub use config::VideoConfig;
pub use party::PartyStamps;

pub struct VideoInner {
    pub core: Core,
    pub identity: Identity,
    pub cfg: VideoConfig,
    party_stamps: RwLock<Option<Arc<dyn PartyStamps>>>,
}

/// The video domain. Cheap to clone.
#[derive(Clone)]
pub struct Video(Arc<VideoInner>);

impl Deref for Video {
    type Target = VideoInner;

    fn deref(&self) -> &VideoInner {
        &self.0
    }
}

impl Video {
    /// Builds the domain and plugs it into the shared registries: subject
    /// resolvers for channels, videos and playlists, the `video` section of
    /// the personal data export, and the guard of `party:` realtime topics
    /// (which must be in place in every process, the gateway's included).
    pub fn new(core: Core, identity: Identity, cfg: VideoConfig) -> anyhow::Result<Self> {
        cfg.validate()?;
        subjects::register(&core);
        core.topic_guards.register("party", party::guard(&core));
        Ok(Self(Arc::new(VideoInner { core, identity, cfg, party_stamps: RwLock::new(None) })))
    }

    /// Registers the seven video services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::video::{
            ChannelServiceExt, PlaybackServiceExt, PlaylistServiceExt, StudioAnalyticsServiceExt, VideoFeedServiceExt,
            VideoServiceExt, WatchPartyServiceExt,
        };
        let me = Arc::new(self.clone());
        let router = ChannelServiceExt::register(me.clone(), router);
        let router = VideoServiceExt::register(me.clone(), router);
        let router = PlaybackServiceExt::register(me.clone(), router);
        let router = PlaylistServiceExt::register(me.clone(), router);
        let router = VideoFeedServiceExt::register(me.clone(), router);
        let router = WatchPartyServiceExt::register(me.clone(), router);
        StudioAnalyticsServiceExt::register(me, router)
    }

    /// Lets watch party chat carry stamps (see [`PartyStamps`]). Call once
    /// at startup; until then stamps in party chat are refused.
    pub fn set_party_stamps(&self, stamps: Arc<dyn PartyStamps>) {
        *self.party_stamps.write() = Some(stamps);
    }

    pub(crate) fn party_stamps(&self) -> Option<Arc<dyn PartyStamps>> {
        self.party_stamps.read().clone()
    }

    /// Applies whether `owner_id`'s account is locked to their videos and
    /// playlists ([`locks::sync_owner`]); the handler of
    /// `social.follow.settings_changed`. Returns how many videos changed.
    pub async fn sync_owner_lock(&self, owner_id: Uuid) -> AppResult<u64> {
        locks::sync_owner(&self.core, owner_id).await
    }
}
