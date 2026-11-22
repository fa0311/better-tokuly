//! Communities (コミュニティ): places for a class, a club, a game, an 推し, a
//! neighbourhood, a study group or a tech topic — with boards, events,
//! announcements and their own safety tools.
//!
//! # For other domains
//!
//! - [`events`] holds the payloads of the `community.*` events this crate
//!   emits (quest counts posts and check-ins, feed shows what friends join).
//! - [`Community::is_member`] and [`Community::role_of`] answer "is this
//!   user in that community?" for domains that gate something on it.
//! - Likes, comments, bookmarks and reports work through the subject types
//!   `COMMUNITY`, `COMMUNITY_THREAD`, `COMMUNITY_REPLY` and `COMMUNITY_EVENT`
//!   (resolvers registered in [`Community::new`]).
//! - [`Community::author_for_moderation`] tells site moderation who is
//!   behind a subject — including a post in an anonymous channel, which
//!   belongs to nobody as far as the subject resolvers say.
//!
//! # Map of the crate
//!
//! | module | what lives there |
//! |---|---|
//! | [`permissions`] | roles and the permission table (pure) |
//! | `access` | who is asking and what they are to a community; every authorization check |
//! | `model` | database rows and the numbers stored in enum columns |
//! | `validate`, `text` | validation; mentions, the word filter, tags (pure) |
//! | [`pseudonym`] | the names of anonymous channels (pure) |
//! | `communities`, `membership` | communities; joining, requests, invites, roles |
//! | `channels`, `threads`, `replies`, `polls`, `posting`, `render` | the boards |
//! | `moderation`, `modlog` | sanctions, the review queue, word filter settings, the log, unmasking |
//! | `retention` | what stays of removed content, for whom, and the purge after the retention period |
//! | `calendar` | events, RSVP with waitlist, check-in |
//! | `discovery` | directory, trending, friends' and school communities |
//! | `*_svc` | the five Connect services: thin handlers over the modules above |
//! | `notices`, `push` | notification texts; realtime pushes |
//! | `subjects`, `profile`, `export` | subject resolvers and the topic guard; the profile card; data export |
//! | `cleanup`, `workers` | reactions to other domains' events; background workers and jobs |
use std::{ops::Deref, sync::Arc, time::Duration};

use tokuly_core::prelude::*;
use tokuly_identity::Identity;
use tokuly_proto::common::SubjectType;

mod access;
mod board_svc;
mod calendar;
mod channels;
mod cleanup;
mod communities;
mod community_svc;
pub mod config;
mod discovery;
mod event_svc;
pub mod events;
mod export;
mod membership;
mod model;
mod moderation;
mod moderation_svc;
mod modlog;
mod notices;
pub mod permissions;
mod polls;
mod posting;
mod profile;
pub mod pseudonym;
mod push;
mod render;
mod replies;
mod retention;
mod subjects;
mod text;
mod threads;
mod validate;
mod workers;

pub use config::CommunityConfig;
pub use permissions::Role;
pub use workers::{AnnounceJob, JOB_ANNOUNCE, JOB_SWEEP, SweepJob};

pub struct CommunityInner {
    pub core: Core,
    pub identity: Identity,
    pub cfg: CommunityConfig,
}

/// The community domain. Cheap to clone.
#[derive(Clone)]
pub struct Community(Arc<CommunityInner>);

impl Deref for Community {
    type Target = CommunityInner;

    fn deref(&self) -> &CommunityInner {
        &self.0
    }
}

impl Community {
    /// Builds the domain and plugs it into the rest: subject resolvers for
    /// its four subject types, the guard of the `community:` realtime
    /// topics, the profile card and the `community` section of the personal
    /// data export.
    pub fn new(core: Core, identity: Identity, cfg: CommunityConfig) -> anyhow::Result<Self> {
        cfg.validate()?;
        let community = Self(Arc::new(CommunityInner { core: core.clone(), identity: identity.clone(), cfg }));
        subjects::register(&community);
        identity.add_profile_extension(Arc::new(community.clone()));
        core.exporters.register(Arc::new(export::CommunityExporter::new(community.clone())));
        Ok(community)
    }

    /// Registers the five community services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::community::{
            CommunityBoardServiceExt, CommunityDiscoveryServiceExt, CommunityEventServiceExt,
            CommunityModerationServiceExt, CommunityServiceExt,
        };
        let me = Arc::new(self.clone());
        let router = CommunityServiceExt::register(me.clone(), router);
        let router = CommunityBoardServiceExt::register(me.clone(), router);
        let router = CommunityModerationServiceExt::register(me.clone(), router);
        let router = CommunityEventServiceExt::register(me.clone(), router);
        CommunityDiscoveryServiceExt::register(me, router)
    }

    /// A rate limit of this domain: at most `max` of `what` per `window` for `who`.
    pub(crate) async fn limit(
        &self,
        what: &str,
        who: impl std::fmt::Display,
        max: u32,
        window: Duration,
    ) -> AppResult<()> {
        self.core.cache.rate_limit(&format!("community:{what}:{who}"), max, window).await
    }

    /// Who answers for a subject of this domain: the author of a thread or
    /// a reply, the organiser of an event, the owner of a community —
    /// removed ones included while they are retained. `None` when there is
    /// no such subject (any more).
    ///
    /// **For site moderation only.** For a post in an anonymous channel this
    /// is the one way to its author besides `RevealAuthor`: the subject
    /// resolvers name nobody, so that nothing a member can do (liking,
    /// blocking, watching counters) depends on who wrote it. Never pass the
    /// answer to anything a member can read.
    pub async fn author_for_moderation(&self, subject_type: SubjectType, subject_id: Uuid) -> AppResult<Option<Uuid>> {
        let query = match subject_type {
            SubjectType::SUBJECT_TYPE_COMMUNITY => {
                "SELECT owner_id FROM communities WHERE id = $1 AND purged_at IS NULL"
            }
            SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD => {
                "SELECT author_id FROM community_threads WHERE id = $1 AND purged_at IS NULL"
            }
            SubjectType::SUBJECT_TYPE_COMMUNITY_REPLY => {
                "SELECT author_id FROM community_replies WHERE id = $1 AND purged_at IS NULL"
            }
            SubjectType::SUBJECT_TYPE_COMMUNITY_EVENT => {
                "SELECT organizer_id FROM community_events WHERE id = $1 AND purged_at IS NULL"
            }
            _ => return Ok(None),
        };
        Ok(sqlx::query_scalar(query).bind(subject_id).fetch_optional(&self.core.db).await?)
    }

    /// Whether `user_id` is a member of the community.
    pub async fn is_member(&self, community_id: Uuid, user_id: Uuid) -> AppResult<bool> {
        Ok(self.role_of(community_id, user_id).await?.is_some())
    }

    /// The role of `user_id` in the community; `None` when they are not a
    /// member or the community is closed.
    pub async fn role_of(&self, community_id: Uuid, user_id: Uuid) -> AppResult<Option<Role>> {
        let role: Option<i32> = sqlx::query_scalar(
            "SELECT m.role FROM community_members m JOIN communities c ON c.id = m.community_id
             WHERE m.community_id = $1 AND m.user_id = $2 AND c.deleted_at IS NULL",
        )
        .bind(community_id)
        .bind(user_id)
        .fetch_optional(&self.core.db)
        .await?;
        Ok(role.and_then(Role::from_code))
    }
}
