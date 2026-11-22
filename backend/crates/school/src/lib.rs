//! School life: timetables, study sessions and study rooms, exams and
//! assignments (`tokuly.school.v1`).
//!
//! Layout:
//! - [`timetable`]: subjects' home is `subjects`; terms, the weekly grid,
//!   one-off changes and day overrides; `timetable::resolve` turns those into
//!   what happens on a date; sharing with friends and the class reminder.
//! - [`study`]: the focus timer (`timer` is the engine it shares with rooms),
//!   the study log, totals, streaks, the heatmap, who is studying, cheers.
//! - [`rooms`]: study rooms, whose members are the study sessions inside them.
//! - `exams`, `assignments`: exam periods and homework with reminders.
//! - `profile`, `hooks`: what the domain plugs into profiles, reports,
//!   realtime topics and the data export; `cleanup` and `workers` react to
//!   other domains and run the periodic sweep.
//!
//! Time is a parameter everywhere: handlers read [`School::now`] once and
//! pass it down, and nothing in the domain asks the database for the time.
//! Days are counted in each user's own time zone (`local`).
//!
//! Other domains learn what happens here from events ([`events`]) and from
//! the profile cards (`school.now`, `school.studying`, `school.exam`).
use std::{
    ops::Deref,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};

use chrono::{DateTime, Utc};
use tokuly_core::prelude::*;
use tokuly_identity::Identity;
use tokuly_proto::{
    realtime::server_frame::Body,
    school::{SchoolEvent, school_event::Event as Push},
};

mod assignments;
mod cleanup;
pub mod config;
pub mod events;
mod exams;
mod hooks;
mod local;
mod profile;
mod rooms;
mod settings;
mod study;
mod subjects;
mod text;
mod timer;
mod timetable;
mod workers;

pub use config::SchoolConfig;
pub use workers::SweepReport;

pub struct SchoolInner {
    pub core: Core,
    pub cfg: SchoolConfig,
    /// Where [`School::set_clock`] stopped the clock, in milliseconds since
    /// the epoch; [`RUNNING`] while it follows the system clock.
    clock_stopped_at_ms: AtomicI64,
    /// Added to the system clock by [`School::shift_clock`].
    clock_offset_ms: AtomicI64,
}

/// `clock_stopped_at_ms` of a clock that runs.
const RUNNING: i64 = i64::MIN;

/// The school domain. Cheap to clone.
#[derive(Clone)]
pub struct School(Arc<SchoolInner>);

impl Deref for School {
    type Target = SchoolInner;

    fn deref(&self) -> &SchoolInner {
        &self.0
    }
}

impl School {
    /// Builds the domain and plugs it into the rest: profile cards, the
    /// subject resolver for study rooms (reports), the guard of the
    /// `study:{room_id}` realtime topic and the `school` section of the data
    /// export.
    pub fn new(core: Core, identity: Identity, cfg: SchoolConfig) -> anyhow::Result<Self> {
        cfg.validate()?;
        let school = Self(Arc::new(SchoolInner {
            core: core.clone(),
            cfg,
            clock_stopped_at_ms: AtomicI64::new(RUNNING),
            clock_offset_ms: AtomicI64::new(0),
        }));
        identity.add_profile_extension(Arc::new(school.clone()));
        hooks::register(&school);
        Ok(school)
    }

    /// Registers this domain's RPC services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::school::{
            AssignmentServiceExt, ExamServiceExt, SchoolServiceExt, StudyRoomServiceExt, StudyServiceExt,
            TimetableServiceExt,
        };
        let me = Arc::new(self.clone());
        let router = SchoolServiceExt::register(me.clone(), router);
        let router = TimetableServiceExt::register(me.clone(), router);
        let router = StudyServiceExt::register(me.clone(), router);
        let router = StudyRoomServiceExt::register(me.clone(), router);
        let router = ExamServiceExt::register(me.clone(), router);
        AssignmentServiceExt::register(me, router)
    }

    /// The domain's clock: the system time in whole milliseconds, which
    /// PostgreSQL stores without rounding, so a timestamp read back equals
    /// the one written.
    pub fn now(&self) -> Time {
        let millis = match self.clock_stopped_at_ms.load(Ordering::Relaxed) {
            RUNNING => Utc::now().timestamp_millis() + self.clock_offset_ms.load(Ordering::Relaxed),
            stopped_at => stopped_at,
        };
        DateTime::from_timestamp_millis(millis).unwrap_or_default()
    }

    /// Stops this instance's clock at `at`. Timers, reminders and day
    /// boundaries all read [`Self::now`], so tests put the clock where their
    /// story needs it and move it with [`Self::shift_clock`] instead of
    /// waiting; nothing in the server calls either.
    #[doc(hidden)]
    pub fn set_clock(&self, at: Time) {
        self.clock_stopped_at_ms.store(at.timestamp_millis(), Ordering::Relaxed);
    }

    /// Moves this instance's clock forward (or back): a stopped clock stays
    /// stopped at the new time, a running one keeps running from it.
    #[doc(hidden)]
    pub fn shift_clock(&self, by: chrono::Duration) {
        if self.clock_stopped_at_ms.load(Ordering::Relaxed) == RUNNING {
            self.clock_offset_ms.fetch_add(by.num_milliseconds(), Ordering::Relaxed);
        } else {
            self.clock_stopped_at_ms.fetch_add(by.num_milliseconds(), Ordering::Relaxed);
        }
    }

    /// Pushes a school event to every connected device of one user.
    pub(crate) async fn push(&self, user_id: Uuid, event: Push) {
        self.core.rt.notify_user(user_id, frame(event)).await;
    }

    /// Pushes the same school event to several users.
    pub(crate) async fn push_many(&self, user_ids: impl IntoIterator<Item = Uuid>, event: Push) {
        self.core.rt.notify_users(user_ids, frame(event)).await;
    }

    /// Pushes a school event to everyone following a study room.
    pub(crate) async fn push_room(&self, room_id: Uuid, event: Push) {
        self.core.rt.notify_topic(&rooms::topic(room_id), frame(event)).await;
    }
}

fn frame(event: Push) -> Body {
    Body::School(Box::new(SchoolEvent { event: Some(event), ..Default::default() }))
}

/// SQL condition: the account in `column` is neither deleted nor suspended.
/// What a suspended account shared stays stored but is shown to nobody, the
/// same as if its owner had stopped sharing it.
pub(crate) fn account_is_active(column: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM users account WHERE account.id = {column} AND account.deleted_at IS NULL AND account.banned_at IS NULL)"
    )
}

/// Serializes changes to one user's set of something (their terms, their
/// active session) for the rest of the transaction.
pub(crate) async fn lock_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    what: &str,
    user_id: Uuid,
) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("school:{what}:{}", user_id.simple()))
        .execute(&mut **tx)
        .await?;
    Ok(())
}
