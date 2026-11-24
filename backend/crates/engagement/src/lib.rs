//! Engagement (コメント・リアクション・通報): comments, reactions, bookmarks,
//! shares and reports on any subject.
//!
//! This domain knows nothing about videos, notes or stamps. Everything hangs
//! off a `tokuly.common.v1.SubjectRef`, and the domain that owns the subject
//! answers — through the `SubjectResolver` it registered in `core.subjects`
//! — who may see it, who owns it and whether it takes interactions.
//!
//! # For other domains
//!
//! - **Get comments, likes, saves, shares and reports on your content** by
//!   registering a `SubjectResolver` for your `SubjectType`. Nothing else is
//!   needed; this crate is never called directly.
//! - **Keep your own counters** by implementing
//!   `SubjectResolver::counter_changed`: it is called after every change
//!   with one of `"comment_count"`, `"like_count"`, `"dislike_count"`,
//!   `"reaction_count"` (emoji, Unicode and custom), `"bookmark_count"`, `"share_count"` and the
//!   delta. `comment_count` counts the comments in public view.
//! - **Follow what users do** through the events in [`events`]
//!   (`engagement.reaction.changed`, `engagement.bookmark.changed`,
//!   `engagement.share.recorded`, `engagement.comment.published`, ...).
//! - **Channel identities**: comments can be written as a channel. The video
//!   domain owns channels and this crate may not depend on it, so the server
//!   hands in a [`ChannelDirectory`] with [`Engagement::set_channel_directory`].
//!   Without one, comments are written as users only.
//!
//! # Map of the crate
//!
//! | module | what lives there |
//! |---|---|
//! | [`subjects`] | resolving subjects for a viewer; the resolvers and the topic guard this domain registers |
//! | [`comments`] | comment rows, visibility, lifecycle, screening, lists, rendering, notifications |
//! | [`reactions`] | likes, dislikes and emoji (Unicode and custom): rules per subject, owners' settings, pictures, the grouped notification and the live pop |
//! | [`bookmarks`], [`shares`], [`reports`] | the other three features |
//! | [`mutes`] | the copy of who muted whom, for "who reacted" |
//! | [`stats`] | a subject's counters and the throttled realtime push of them |
//! | [`effects`] | what a committed change still owes the outside world |
//! | [`text`] | every decision made by looking at text (pure functions) |
//! | `*_svc` | the six Connect services: thin handlers over the modules above |
//! | [`cleanup`], [`workers`], [`export`] | reactions to other domains' events; personal data export |
//!
//! # Invariants worth knowing
//!
//! - Inside a transaction only the transaction's connection is used: what
//!   needs other lookups (subject resolution, audiences, senders) is worked
//!   out before it opens.
//! - Counters move in the transaction that changes what they count; realtime
//!   pushes and counter callbacks happen after the commit ([`effects`]).
//! - Locks are taken in one order: comments (the top-level comment before
//!   its replies), bookmark collections by ID, the rows being counted
//!   (reactions — after the per-user-and-subject lock and, for an emoji, the
//!   subject's emoji lock — bookmarks, per-emoji and per-target counts),
//!   and last the
//!   subject's statistics row. Transactions that change many comments lock
//!   every thread involved first, in ID order
//!   ([`comments::lock_threads_of`]).
use std::{collections::HashMap, ops::Deref, sync::Arc};

use async_trait::async_trait;
use parking_lot::RwLock;
use tokuly_core::prelude::*;
use tokuly_identity::Identity;
use tokuly_proto::common::ChannelSummary;

pub mod bookmarks;
pub mod cleanup;
pub mod comments;
pub mod config;
pub mod effects;
pub mod events;
pub mod export;
pub mod mutes;
pub mod reactions;
pub mod reports;
pub mod shares;
pub mod stats;
pub mod subjects;
pub mod text;
pub mod workers;

mod bookmark_svc;
mod comment_svc;
mod engagement_svc;
mod moderation_svc;
mod reaction_svc;
mod report_svc;

pub use config::EngagementConfig;
pub use subjects::SubjectKey;

/// The page of a list request, with the list's own default and maximum size.
pub(crate) fn page_of(page: Option<&tokuly_proto::common::PageRequest>, default: i64, max: i64) -> Page {
    match page {
        Some(page) => Page::new(page.limit, &page.cursor, default, max),
        None => Page::new(0, "", default, max),
    }
}

/// Channels, as far as this domain needs them: the identities a comment can
/// be written with. Implemented by the video domain and wired by the server.
#[async_trait]
pub trait ChannelDirectory: Send + Sync + 'static {
    /// The channel, if `user_id` owns it and may act as it.
    async fn owned_channel(&self, user_id: Uuid, channel_id: Uuid) -> AppResult<Option<ChannelSummary>>;

    /// Summaries for display, keyed by channel ID. Unknown IDs are absent.
    async fn summaries(&self, ids: &[Uuid]) -> AppResult<HashMap<Uuid, ChannelSummary>>;
}

pub struct EngagementInner {
    pub core: Core,
    pub identity: Identity,
    pub cfg: EngagementConfig,
    /// The subject types that take several emoji per person
    /// (`TOKULY_ENGAGEMENT_MULTI_REACTION_TYPES`, as numbers).
    multi_reaction_types: Vec<i32>,
    channels: RwLock<Option<Arc<dyn ChannelDirectory>>>,
}

/// The engagement domain. Cheap to clone.
#[derive(Clone)]
pub struct Engagement(Arc<EngagementInner>);

impl Deref for Engagement {
    type Target = EngagementInner;

    fn deref(&self) -> &EngagementInner {
        &self.0
    }
}

impl Engagement {
    /// Creates the domain and registers what it contributes to the shared
    /// registries: the subject resolver for comments, a fallback resolver
    /// for users (when no other domain registered one before), the guard of
    /// the `subject:` realtime topics and the personal data exporter.
    pub fn new(core: Core, identity: Identity, cfg: EngagementConfig) -> anyhow::Result<Self> {
        anyhow::ensure!(
            cfg.comments_per_minute > 0 && cfg.comments_per_hour > 0 && cfg.reports_per_hour > 0,
            "the engagement rate limits must be positive"
        );
        anyhow::ensure!(
            cfg.max_collections > 0 && cfg.max_bookmarks > 0,
            "TOKULY_ENGAGEMENT_MAX_COLLECTIONS and TOKULY_ENGAGEMENT_MAX_BOOKMARKS must be positive"
        );
        anyhow::ensure!(cfg.report_repeat_days >= 0, "TOKULY_ENGAGEMENT_REPORT_REPEAT_DAYS must not be negative");
        anyhow::ensure!(
            cfg.reaction_emoji_per_subject > 0 && cfg.reactions_per_person > 0,
            "TOKULY_ENGAGEMENT_REACTION_EMOJI_PER_SUBJECT and TOKULY_ENGAGEMENT_REACTIONS_PER_PERSON must be positive"
        );
        let multi_reaction_types = cfg.multi_reaction_type_numbers()?;
        let engagement =
            Self(Arc::new(EngagementInner { core, identity, cfg, multi_reaction_types, channels: RwLock::new(None) }));
        subjects::register(&engagement);
        export::register(&engagement.core);
        Ok(engagement)
    }

    /// Registers the six engagement services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::engagement::{
            BookmarkServiceExt, CommentModerationServiceExt, CommentServiceExt, EngagementServiceExt,
            ReactionServiceExt, ReportServiceExt,
        };
        let me = Arc::new(self.clone());
        let router = CommentServiceExt::register(me.clone(), router);
        let router = CommentModerationServiceExt::register(me.clone(), router);
        let router = ReactionServiceExt::register(me.clone(), router);
        let router = BookmarkServiceExt::register(me.clone(), router);
        let router = EngagementServiceExt::register(me.clone(), router);
        ReportServiceExt::register(me, router)
    }

    /// Makes channel identities available to comments. Call once at
    /// startup, with the video domain's implementation.
    pub fn set_channel_directory(&self, directory: Arc<dyn ChannelDirectory>) {
        *self.channels.write() = Some(directory);
    }

    fn channel_directory(&self) -> Option<Arc<dyn ChannelDirectory>> {
        self.channels.read().clone()
    }

    /// Summaries of channels for display; empty without a directory.
    pub(crate) async fn channel_summaries(&self, channel_ids: &[Uuid]) -> AppResult<HashMap<Uuid, ChannelSummary>> {
        match self.channel_directory() {
            Some(directory) if !channel_ids.is_empty() => directory.summaries(channel_ids).await,
            _ => Ok(HashMap::new()),
        }
    }

    /// The channel `user_id` wants to comment as: `CHANNEL_NOT_FOUND` unless
    /// it exists and is theirs (and always without a directory).
    pub(crate) async fn owned_channel(&self, user_id: Uuid, channel_id: Uuid) -> AppResult<ChannelSummary> {
        let channel = match self.channel_directory() {
            Some(directory) => directory.owned_channel(user_id, channel_id).await?,
            None => None,
        };
        channel.ok_or_else(|| AppError::not_found("CHANNEL_NOT_FOUND", "channel not found"))
    }
}
