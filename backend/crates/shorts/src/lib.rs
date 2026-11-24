//! Shorts (ショート): vertical videos of at most 180 seconds on the channels
//! of the video domain, the swipe feeds that play them, hashtag and sound
//! pages, and creator analytics.
//!
//! # For other domains
//!
//! - [`shorts::find`] / [`shorts::ShortRow`] resolve a short by UUID or short ID;
//! - [`access`] holds the visibility rules ([`access::can_watch`] with the
//!   video domain's [`access::Viewer`]);
//! - [`ClipDirectory`] / [`Shorts::set_clip_directory`]: how the server
//!   lends the live domain's clips to "create a short from a clip" (this
//!   crate must not depend on the live crate);
//! - [`events`] has the payloads of `shorts.short.*`.
//!
//! # Map of the crate
//!
//! | module | what lives there |
//! |---|---|
//! | [`access`] | every rule about who may see what, as code and as SQL predicates |
//! | [`text`] | captions: length, hashtags, mentions |
//! | [`shorts`] | rows, creation, editing, deletion, the processing state machine, pins |
//! | [`listing`] | what follows from a short being public: hashtag and sound counts, search, announcements, mention notifications |
//! | [`sounds`], [`hashtags`] | sounds and hashtags: lookups, pages, suggestions |
//! | [`cover`] | cutting a cover frame out of the video |
//! | [`hydrate`] | rows to API messages for a viewer |
//! | [`library`] | what viewers keep: saves, the copy of their likes, "not interested", shares |
//! | [`signals`] | playback reports: views, the Valkey buffer and its flush, the seen list |
//! | [`ranking`] | the for-you score: formula, candidates, personal ranking, the ranking worker |
//! | [`feeds`] | the three feeds |
//! | [`snapshot`] | paging through lists ordered by a value that moves (top, most viewed) |
//! | [`analytics`] | creator analytics from ClickHouse |
//! | `*_svc` | the four Connect services: handlers over the modules above |
//! | [`workers`] | event consumers, jobs and timers |
//! | [`subjects`] | the subject resolver (comments, likes, reports) and data export |
//! | [`events`] | topics and payloads of the events emitted and consumed, job payloads |
use std::{ops::Deref, sync::Arc};

use async_trait::async_trait;
use parking_lot::RwLock;
use tokuly_core::{Core, prelude::*};
use tokuly_identity::Identity;

pub mod access;
pub mod analytics;
pub mod config;
pub mod cover;
pub mod events;
pub mod feeds;
pub mod hashtags;
pub mod hydrate;
pub mod library;
pub mod listing;
pub mod locks;
pub mod ranking;
pub mod shorts;
pub mod signals;
pub mod snapshot;
pub mod sounds;
pub mod subjects;
pub mod text;
pub mod workers;

mod analytics_svc;
mod discovery_svc;
mod feed_svc;
mod short_svc;

pub use config::ShortsConfig;

/// A clip as far as "create a short from a clip" needs to know it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipMedia {
    pub clip_id: Uuid,
    /// The media asset the clip plays.
    pub asset_id: Uuid,
}

/// Clips belong to the live domain, which this crate must not depend on.
/// The server implements this with the live domain's API and hands it over
/// with [`Shorts::set_clip_directory`].
#[async_trait]
pub trait ClipDirectory: Send + Sync + 'static {
    /// The clip named `clip_id` (UUID or short ID, as clients send it) if
    /// `user_id` created it; `None` when there is no such clip or it is
    /// someone else's.
    async fn clip_of(&self, user_id: Uuid, clip_id: &str) -> AppResult<Option<ClipMedia>>;
}

pub struct ShortsInner {
    pub core: Core,
    pub identity: Identity,
    pub cfg: ShortsConfig,
    clips: RwLock<Option<Arc<dyn ClipDirectory>>>,
}

/// The shorts domain. Cheap to clone.
#[derive(Clone)]
pub struct Shorts(Arc<ShortsInner>);

impl Deref for Shorts {
    type Target = ShortsInner;

    fn deref(&self) -> &ShortsInner {
        &self.0
    }
}

impl Shorts {
    /// Builds the domain and plugs it into the shared registries: the
    /// subject resolver for shorts and the `shorts` section of the personal
    /// data export.
    pub fn new(core: Core, identity: Identity, cfg: ShortsConfig) -> anyhow::Result<Self> {
        cfg.validate()?;
        subjects::register(&core);
        Ok(Self(Arc::new(ShortsInner { core, identity, cfg, clips: RwLock::new(None) })))
    }

    /// Registers the four shorts services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::shorts::{
            ShortAnalyticsServiceExt, ShortDiscoveryServiceExt, ShortFeedServiceExt, ShortServiceExt,
        };
        let me = Arc::new(self.clone());
        let router = ShortServiceExt::register(me.clone(), router);
        let router = ShortFeedServiceExt::register(me.clone(), router);
        let router = ShortDiscoveryServiceExt::register(me.clone(), router);
        ShortAnalyticsServiceExt::register(me, router)
    }

    /// Lets shorts be made from clips that are not the caller's own upload
    /// (see [`ClipDirectory`]). Call once at startup; until then only the
    /// caller's own media is accepted.
    pub fn set_clip_directory(&self, clips: Arc<dyn ClipDirectory>) {
        *self.clips.write() = Some(clips);
    }

    pub(crate) fn clip_directory(&self) -> Option<Arc<dyn ClipDirectory>> {
        self.clips.read().clone()
    }
}
