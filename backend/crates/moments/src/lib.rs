//! Moments (いまの瞬間): once a day, everyone in a region is told at the same
//! random time to take a moment — back and front camera at once — within
//! two minutes; friends' moments stay blurred until you post yours.
//!
//! A **region** is a time zone (`users.time_zone`). Its days are drawn a
//! day ahead (`moment_days`, a random time inside the local window) and go
//! out exactly once, through the `moments.prompt` job: the announcement
//! starts the day, the fan-out tells the region page by page, each page
//! committed with its cursor. A moment belongs to its owner's current day
//! (one per day); friends see each other's moments of their current days
//! only, locked until they posted their own. Past days stay in the owner's
//! calendar.
//!
//! # For other domains
//!
//! [`events`] holds the payloads of the `moments.*` events. Moments and
//! reactions are subjects (`SUBJECT_TYPE_MOMENT`, `_MOMENT_REACTION`) that
//! can be reported; they take no comments.
//!
//! # Map of the crate
//!
//! | module | what lives there |
//! |---|---|
//! | [`rules`] | everything free of I/O: prompt times, lateness, validation, placeholders |
//! | `model` | database rows, stored numbers, media holds |
//! | `days` | drawing days ahead, the prompt job, the current day of a region |
//! | `posting` | posting and deleting moments |
//! | `viewing` | who sees which moment, locked or not, with which location; rendering |
//! | `reactions` | RealMoji and stamp reactions, the RealMoji library |
//! | `settings` | per-account settings and the defaults by age group |
//! | `calendar` | the owner's month |
//! | `cleanup` | other domains' events, the purge after the evidence period |
//! | `subjects` | subject resolvers and the personal data export |
//! | `copy` | notification texts |
//! | `*_svc` | the Connect services: thin handlers over the modules above |
//! | `workers` | background work |
use std::{
    ops::Deref,
    sync::{Arc, Mutex},
    time::Instant,
};

use tokuly_core::Core;
use tokuly_identity::Identity;
use tokuly_proto::moments::{MomentsEvent, moments_event::Event as Push};
use uuid::Uuid;

pub mod config;
pub mod events;
pub mod rules;

mod calendar;
mod cleanup;
mod copy;
mod days;
mod model;
mod moment_svc;
mod posting;
mod reaction_svc;
mod reactions;
mod settings;
mod subjects;
mod viewing;
mod workers;

pub use config::MomentsConfig;
pub use copy::{KIND_PROMPT, KIND_REACTION};
pub use days::{JOB_PROMPT, MAX_DELAY, PromptJob, Sent};

pub struct MomentsInner {
    pub core: Core,
    pub identity: Identity,
    pub cfg: MomentsConfig,
    /// When the planner last looked for new regions (see `days`).
    regions_refreshed: Mutex<Option<Instant>>,
}

/// The moments domain. Cheap to clone.
#[derive(Clone)]
pub struct Moments(Arc<MomentsInner>);

impl Deref for Moments {
    type Target = MomentsInner;

    fn deref(&self) -> &MomentsInner {
        &self.0
    }
}

impl Moments {
    /// Builds the domain and plugs it into the generic domains: moments and
    /// their reactions become reportable subjects, and personal data
    /// exports gain a `moments` section.
    pub fn new(core: Core, identity: Identity, cfg: MomentsConfig) -> anyhow::Result<Self> {
        cfg.validate()?;
        let moments = Self(Arc::new(MomentsInner { core, identity, cfg, regions_refreshed: Mutex::new(None) }));
        subjects::register(&moments);
        Ok(moments)
    }

    /// Registers the two services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::moments::{MomentReactionServiceExt, MomentServiceExt};
        let me = Arc::new(self.clone());
        let router = MomentServiceExt::register(me.clone(), router);
        MomentReactionServiceExt::register(me, router)
    }

    /// Pushes a moments event to every connected device of the given users.
    pub(crate) async fn push(&self, user_ids: impl IntoIterator<Item = Uuid>, event: Push) {
        self.core.rt.notify_users(user_ids, &MomentsEvent { event: Some(event), ..Default::default() }).await;
    }
}
