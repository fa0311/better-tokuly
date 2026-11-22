//! Free-time status, plans with RSVP and cost split, date polls, countdowns and anniversaries.
//!
//! How a group of friends gets from "ひま〜" to actually meeting. Everything
//! is among friends; nothing here is public.
//!
//! Layout:
//! - `free` — the "ひま" status, "あそぼ!" answers and "tell me when they are free".
//! - `plan`, `participants`, `checklist`, `expenses` — plans: invitations,
//!   RSVP with a waitlist, the checklist and the cost split. The arithmetic
//!   of the split lives in `split`, free of any I/O.
//! - `polls` — date polls that end in a plan.
//! - `countdowns`, `anniversaries` (milestone dates in `milestones`),
//!   `birthdays` — everything that counts days.
//! - `calendar` — one month of all of the above.
//! - `*_svc` — the RPC services as thin handlers.
//! - `profile` adds cards to identity's profiles, `subject` lets comments
//!   and reports attach to a plan, `maintenance` reacts to other domains'
//!   events, runs the reminders and exports personal data.
//! - `people` answers who is a friend and how someone is named; `dates`,
//!   `input` and `copy` hold time-zone helpers, validation and every text
//!   the server writes.
//!
//! [`events`] holds the payloads of the events this domain emits.
use std::{ops::Deref, sync::Arc, time::Duration};

use tokuly_core::prelude::*;
use tokuly_identity::Identity;
use tokuly_proto::{
    common::SubjectType,
    plans::{PlansEvent, plans_event::Event as Push},
};
use tokuly_social::Social;

mod anniversaries;
mod anniversary_svc;
mod birthdays;
mod calendar;
mod checklist;
pub mod config;
mod copy;
mod countdowns;
mod dates;
pub mod events;
mod expenses;
mod free;
mod free_svc;
mod input;
mod maintenance;
mod milestones;
mod participants;
mod people;
mod plan;
mod plan_svc;
mod poll_svc;
mod polls;
mod profile;
mod split;
mod subject;

pub use config::PlansConfig;
pub use maintenance::WorkDone;

pub struct PlansInner {
    pub core: Core,
    pub cfg: PlansConfig,
    /// Asked about friendships (see `people`).
    pub(crate) social: Social,
}

/// The plans domain. Cheap to clone.
#[derive(Clone)]
pub struct Plans(Arc<PlansInner>);

impl Deref for Plans {
    type Target = PlansInner;

    fn deref(&self) -> &PlansInner {
        &self.0
    }
}

impl Plans {
    /// Builds the domain and plugs it into the rest of the backend: profiles
    /// gain the "free", countdown and couple cards, plans become a subject
    /// comments and reports can attach to (`SUBJECT_TYPE_PLAN`), and the
    /// personal data export gains a `plans` section.
    ///
    /// `social` is the social domain: besides its `graph` functions, this
    /// domain asks it who someone's best friends are and since when two
    /// people are friends.
    pub fn new(core: Core, identity: Identity, cfg: PlansConfig, social: Social) -> anyhow::Result<Self> {
        cfg.validate()?;
        let plans = Self(Arc::new(PlansInner { core: core.clone(), cfg, social }));
        identity.add_profile_extension(Arc::new(plans.clone()));
        core.subjects.register(SubjectType::SUBJECT_TYPE_PLAN, Arc::new(plans.clone()));
        core.exporters.register(Arc::new(maintenance::PlansExporter::new(core.clone())));
        Ok(plans)
    }

    /// Registers this domain's RPC services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::plans::{
            AnniversaryServiceExt, CalendarServiceExt, FreeServiceExt, PlanServiceExt, PollServiceExt,
        };
        let me = Arc::new(self.clone());
        let router = FreeServiceExt::register(me.clone(), router);
        let router = PlanServiceExt::register(me.clone(), router);
        let router = PollServiceExt::register(me.clone(), router);
        let router = AnniversaryServiceExt::register(me.clone(), router);
        CalendarServiceExt::register(me, router)
    }

    /// Starts the background work: the event consumer (deleted accounts,
    /// ended friendships, blocks, moderation) and the worker that sends
    /// whatever has become due (see [`Self::run_due_work`]).
    pub fn spawn_workers(&self) {
        let plans = self.clone();
        tokio::spawn(async move { plans.run_event_consumer().await });

        let plans = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(plans.cfg.worker_interval_seconds));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = plans.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        if let Err(e) = plans.run_due_work().await {
                            tracing::error!(error = %e, "plans worker pass failed");
                        }
                    }
                }
            }
        });
    }

    /// Pushes a plans event to every connected device of the given users.
    pub(crate) async fn push(&self, user_ids: impl IntoIterator<Item = Uuid>, event: Push) {
        self.core.rt.notify_users(user_ids, &PlansEvent { event: Some(event), ..Default::default() }).await;
    }
}
