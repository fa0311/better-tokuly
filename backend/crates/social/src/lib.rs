//! Friend graph, follows, circles, invites, nearby discovery, bumps, buzzes
//! and map reactions.
//!
//! Layout:
//! - [`graph`] is the API other domains call (who is friends with whom, who
//!   follows whom, who is blocked, interaction reporting), with
//!   [`audience`] for "may this viewer see that content" over every audience
//!   (public, followers, friends, close friends, circles).
//! - `friend_svc`, `interaction_svc` and `follow_svc` (follows and circles)
//!   hold the RPC services as thin handlers; the behaviour lives in one
//!   module per feature (`requests`, `friends`, `blocks`, `best_friends`,
//!   `suggestions`, `invites`, `nearby`, `bumps`, `signals`, `map_reactions`,
//!   `map_views`, `follows`, [`follow_suggestions`], `circles`).
//! - [`claims`] checks positions the client claims (nearby, bumps) through
//!   a [`claims::PositionCheck`] the geo domain plugs in.
//! - `profile` plugs relationship data into identity's profiles.
//! - `presence`, `best_friends` and `maintenance` run the background work.
use std::{
    ops::Deref,
    sync::{Arc, RwLock},
    time::Duration,
};

use tokuly_core::prelude::*;
use tokuly_identity::Identity;
use tokuly_proto::social::{SocialEvent, social_event::Event as Push};

pub mod audience;
mod best_friends;
mod blocks;
pub mod bumps;
mod circles;
pub mod claims;
pub mod config;
mod copy;
pub mod events;
pub mod follow_suggestions;
mod follow_svc;
mod follows;
mod friend_svc;
mod friends;
pub mod graph;
mod idcache;
mod interaction_svc;
mod invites;
mod livegeo;
mod maintenance;
mod map_reactions;
mod map_views;
mod nearby;
mod people;
mod presence;
mod profile;
mod ranking;
mod requests;
mod signals;
pub mod suggestions;

pub use config::SocialConfig;
pub use graph::InteractionKind;

pub struct SocialInner {
    pub core: Core,
    pub cfg: SocialConfig,
    /// Checks positions claimed for nearby discovery and bumps (see [`claims`]).
    position_check: claims::Slot,
    /// Where follow suggestions come from (see [`follow_suggestions`]).
    follow_signals: RwLock<Vec<Arc<dyn follow_suggestions::FollowSignal>>>,
}

/// The social domain. Cheap to clone.
#[derive(Clone)]
pub struct Social(Arc<SocialInner>);

impl Deref for Social {
    type Target = SocialInner;

    fn deref(&self) -> &SocialInner {
        &self.0
    }
}

impl Social {
    /// Builds the domain and plugs it into identity: profiles and user search
    /// results gain relationship data, and the personal data export gains a
    /// `social` section.
    pub fn new(core: Core, identity: Identity, cfg: SocialConfig) -> anyhow::Result<Self> {
        anyhow::ensure!(cfg.max_friends > 0, "TOKULY_SOCIAL_MAX_FRIENDS must be positive");
        anyhow::ensure!(cfg.max_following > 0, "TOKULY_SOCIAL_MAX_FOLLOWING must be positive");
        anyhow::ensure!(cfg.best_friend_count > 0, "TOKULY_SOCIAL_BEST_FRIEND_COUNT must be positive");
        anyhow::ensure!(cfg.ranking_interval_seconds > 0, "TOKULY_SOCIAL_RANKING_INTERVAL_SECONDS must be positive");
        anyhow::ensure!(cfg.bump_window_ms > 0, "TOKULY_SOCIAL_BUMP_WINDOW_MS must be positive");
        for (name, meters) in [
            ("TOKULY_SOCIAL_NEARBY_RADIUS_METERS", cfg.nearby_radius_meters),
            ("TOKULY_SOCIAL_BUMP_RADIUS_METERS", cfg.bump_radius_meters),
            ("TOKULY_SOCIAL_BUZZ_RADIUS_METERS", cfg.buzz_radius_meters),
        ] {
            anyhow::ensure!(meters.is_finite() && meters > 0.0, "{name} must be a positive number of meters");
        }
        let social = Self(Arc::new(SocialInner {
            core: core.clone(),
            cfg,
            position_check: claims::Slot::new(),
            follow_signals: RwLock::new(follow_suggestions::built_in()),
        }));
        identity.add_profile_extension(Arc::new(social.clone()));
        core.exporters.register(Arc::new(maintenance::SocialExporter::new(core.clone())));
        Ok(social)
    }

    /// Registers this domain's RPC services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::social::{CircleServiceExt, FollowServiceExt, FriendServiceExt, InteractionServiceExt};
        let me = Arc::new(self.clone());
        let router = FriendServiceExt::register(me.clone(), router);
        let router = InteractionServiceExt::register(me.clone(), router);
        let router = FollowServiceExt::register(me.clone(), router);
        CircleServiceExt::register(me, router)
    }

    /// Adds a source of follow suggestions (another domain's signal, such as
    /// "popular in your communities"; see [`follow_suggestions`]).
    pub fn add_follow_signal(&self, signal: Arc<dyn follow_suggestions::FollowSignal>) {
        self.follow_signals.write().unwrap_or_else(|poisoned| poisoned.into_inner()).push(signal);
    }

    fn follow_signals(&self) -> Vec<Arc<dyn follow_suggestions::FollowSignal>> {
        self.follow_signals.read().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    /// Starts the background work: presence fan-out to friends, the
    /// best-friend ranking, the event consumer (sign-ups through invites,
    /// account deletions, privacy changes) and periodic pruning.
    pub fn spawn_workers(&self) {
        let social = self.clone();
        tokio::spawn(async move { social.run_presence_fanout().await });

        let social = self.clone();
        tokio::spawn(async move { social.run_event_consumer().await });

        let social = self.clone();
        let every = Duration::from_secs(self.cfg.ranking_interval_seconds);
        tokio::spawn(async move {
            social
                .run_periodically("best-friend ranking", every, |social| async move {
                    social.rescore_pending().await.map(|_| ())
                })
                .await
        });

        let social = self.clone();
        tokio::spawn(async move {
            social
                .run_periodically("social maintenance", maintenance::PRUNE_INTERVAL, |social| async move {
                    social.prune().await
                })
                .await
        });
    }

    /// Runs `job` every `every` until shutdown, logging failures.
    async fn run_periodically<F, Fut>(&self, name: &'static str, every: Duration, job: F)
    where
        F: Fn(Social) -> Fut,
        Fut: Future<Output = AppResult<()>>,
    {
        let mut tick = tokio::time::interval(every);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = self.core.shutdown.cancelled() => break,
                _ = tick.tick() => {
                    if let Err(e) = job(self.clone()).await {
                        tracing::error!(job = name, error = %e, "periodic job failed");
                    }
                }
            }
        }
    }

    /// Pushes a social event to every connected device of one user.
    pub(crate) async fn push(&self, user_id: Uuid, event: Push) {
        self.core.rt.notify_user(user_id, &social_event(event)).await;
    }

    /// Pushes the same social event to several users.
    pub(crate) async fn push_many(&self, user_ids: impl IntoIterator<Item = Uuid>, event: Push) {
        self.core.rt.notify_users(user_ids, &social_event(event)).await;
    }

    /// Fails with `NOT_FRIENDS` unless `other` is a friend of `user_id`.
    /// Everything aimed at "a friend" starts with this check; it also covers
    /// unknown, deleted and blocked users, none of whom can be a friend.
    pub(crate) async fn require_friend(&self, user_id: Uuid, other: Uuid) -> AppResult<()> {
        if graph::are_friends(&self.core, user_id, other).await? { Ok(()) } else { Err(not_friends()) }
    }
}

/// The error for anything that needs a friendship which is not there.
pub(crate) fn not_friends() -> AppError {
    AppError::precondition("NOT_FRIENDS", "you are not friends with this user")
}

fn social_event(event: Push) -> SocialEvent {
    SocialEvent { event: Some(event), ..Default::default() }
}
