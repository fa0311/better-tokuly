//! Moments (いまの瞬間): once a day, everyone in a region is told at the same
//! random time to take a moment — back and front camera at once — within
//! two minutes; friends' moments stay blurred until you post yours.
//!
//! A **region** is a band of UTC offsets (`rules::BANDS`), the one of the
//! account's time zone. Its days are drawn a day ahead (`moment_days`, a
//! random time inside the window the band shares) and go out exactly once,
//! through the `moments.prompt` job: the announcement starts the day, the
//! fan-out moves the region's accounts to it page by page, each page
//! committed with its cursor. Every account has a day of its own
//! (`moment_people`): the latest prompt that reached it, of a later date
//! than the one before, so one prompt and one moment per date whatever the
//! time zone does. Friends see each other's moments of their own days only,
//! locked until they posted their own. Past days stay in the owner's
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
//! | [`rules`] | everything free of I/O: the bands, prompt times, lateness, validation, placeholders |
//! | `model` | database rows, stored numbers, media holds |
//! | `days` | the bands' days drawn ahead, the prompt job, everyone's own day |
//! | `posting` | posting and deleting moments |
//! | `viewing` | who sees which moment, locked or not, with which location; rendering |
//! | `reactions` | RealMoji and stamp reactions, the RealMoji library |
//! | `settings` | per-account settings and the defaults by age group |
//! | `calendar` | the owner's month |
//! | `staff` | admins' view of the regions, a prompt sent now |
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

mod admin_svc;
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
mod staff;
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
    /// When the planner last mapped the time zones to their bands (see `days`).
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

    /// Registers the three services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::moments::{MomentAdminServiceExt, MomentReactionServiceExt, MomentServiceExt};
        let me = Arc::new(self.clone());
        let router = MomentServiceExt::register(me.clone(), router);
        let router = MomentReactionServiceExt::register(me.clone(), router);
        MomentAdminServiceExt::register(me, router)
    }

    /// Pushes a moments event to every connected device of the given users.
    pub(crate) async fn push(&self, user_ids: impl IntoIterator<Item = Uuid>, event: Push) {
        self.core.rt.notify_users(user_ids, &MomentsEvent { event: Some(event), ..Default::default() }).await;
    }
}
