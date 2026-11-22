//! Plans (あそぶ予定): what a plan is, who may do what with it, and how it
//! is created, changed, cancelled, listed and shown.
//!
//! A plan is visible to its participants only. [`Access`] is the entry point
//! of every operation: it loads the plan together with the caller's own
//! participation and answers the authorization questions (host, organizer,
//! going). People join and answer in `participants`; the checklist and the
//! cost split have their own modules.
use std::{
    collections::{HashMap, HashSet},
    time::Duration as StdDuration,
};

use buffa::Enumeration as _;
use chrono::{Duration, NaiveDate, TimeZone as _, Utc};
use chrono_tz::Tz;
use sqlx::PgConnection;
use tokuly_core::{
    events as bus,
    outbound::{self, ActivityAudience, ActivityEntry, NotificationRequest},
    prelude::*,
};
use tokuly_identity::users;
use tokuly_proto::{
    common::{LatLng, PageInfo, SubjectType},
    plans::{plan_time, plans_event::Event as Push, *},
};
use tokuly_social::graph;

use crate::{
    Plans,
    checklist::ChecklistRow,
    copy, dates,
    events::{PLAN_CANCELLED, PLAN_CREATED, PlanCancelled, PlanCreated},
    input, people,
};

const MAX_TITLE: usize = 80;
const MAX_DESCRIPTION: usize = 2000;
const MAX_PLACE_NAME: usize = 80;
const MAX_PLACE_ADDRESS: usize = 200;
const MAX_PLACE_ID: usize = 100;
const MAX_CANCEL_REASON: usize = 200;
const MAX_CAPACITY: i32 = 10_000;
/// Longest "minutes before" reminder: a week.
const MAX_REMIND_MINUTES: i32 = 10_080;
const DEFAULT_REMIND_MINUTES: i32 = 60;
const DEFAULT_CURRENCY: &str = "JPY";
/// A plan with a start time and no end counts as over this long after it starts.
const DEFAULT_LENGTH_HOURS: i64 = 3;
/// A plan may be written down after the fact (to split the bill), but not
/// long after; and not absurdly far ahead.
const MAX_PAST_DAYS: i64 = 31;
const MAX_AHEAD_DAYS: i64 = 3 * 366;
const MAX_LENGTH_DAYS: i64 = 31;
/// Changes to their plans (and cancellations) one organizer may make in an hour.
const CHANGES_PER_HOUR: u32 = 60;
/// Changes of time or place one plan may see in an hour: each of them
/// notifies everyone in it.
const ANNOUNCED_CHANGES_PER_HOUR: u32 = 6;
/// People shown on a plan in a list, besides the host.
const PREVIEW_PARTICIPANTS: usize = 5;

/// Path of a plan in the app.
pub(crate) fn plan_path(plan_id: Uuid) -> String {
    format!("/plans/{plan_id}")
}

/// Sort position of plans whose time is to be decided: after every dated one.
fn far_future() -> Time {
    Utc.with_ymd_and_hms(9999, 1, 1, 0, 0, 0).single().expect("a valid date")
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct PlanRow {
    pub id: Uuid,
    pub host_id: Uuid,
    pub title: String,
    pub emoji: String,
    pub description: String,
    pub start_at: Option<Time>,
    pub start_date: Option<NaiveDate>,
    pub end_at: Option<Time>,
    pub end_date: Option<NaiveDate>,
    pub time_zone: String,
    pub sort_at: Option<Time>,
    pub over_at: Option<Time>,
    pub place_name: String,
    pub place_lat: Option<f64>,
    pub place_lng: Option<f64>,
    pub place_id: String,
    pub place_address: String,
    pub guests_can_invite: bool,
    pub capacity: i32,
    pub remind_day_before: bool,
    pub remind_minutes_before: i32,
    pub currency: String,
    pub status: i16,
    pub cancelled_at: Option<Time>,
    pub cancel_reason: String,
    pub poll_id: Option<Uuid>,
    pub comment_count: i32,
    pub created_at: Time,
    pub updated_at: Time,
}

macro_rules! plan_columns {
    () => {
        "p.id, p.host_id, p.title, p.emoji, p.description, p.start_at, p.start_date, p.end_at, p.end_date, p.time_zone, \
         p.sort_at, p.over_at, p.place_name, p.place_lat, p.place_lng, p.place_id, p.place_address, p.guests_can_invite, p.capacity, \
         p.remind_day_before, p.remind_minutes_before, p.currency, p.status, p.cancelled_at, p.cancel_reason, p.poll_id, p.comment_count, \
         p.created_at, p.updated_at"
    };
}
pub(crate) use plan_columns;

impl PlanRow {
    pub(crate) fn is_active(&self) -> bool {
        self.status == PlanStatus::PLAN_STATUS_ACTIVE as i32 as i16
    }

    pub(crate) fn is_over(&self, now: Time) -> bool {
        self.over_at.is_some_and(|over| over <= now)
    }

    pub(crate) fn schedule(&self) -> Schedule {
        let when = match (self.start_at, self.start_date) {
            (Some(start), _) => When::At { start, end: self.end_at },
            (None, Some(start)) => When::On { start, end: self.end_date },
            (None, None) => When::Undecided,
        };
        Schedule { when, zone: dates::zone(&self.time_zone) }
    }

    /// The plan as notification texts name it.
    pub(crate) fn name(&self) -> String {
        copy::plan_name(&self.emoji, &self.title)
    }

    fn place(&self) -> Option<PlanPlace> {
        (!self.place_name.is_empty()).then(|| PlanPlace {
            name: self.place_name.clone(),
            location: self
                .place_lat
                .zip(self.place_lng)
                .map(|(lat, lng)| LatLng { lat, lng, ..Default::default() })
                .into(),
            place_id: self.place_id.clone(),
            address: self.place_address.clone(),
            ..Default::default()
        })
    }
}

/// When a plan happens.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum When {
    /// 未定.
    Undecided,
    /// At a time, with an optional end.
    At { start: Time, end: Option<Time> },
    /// On a day (or from one day to another), without a time.
    On { start: NaiveDate, end: Option<NaiveDate> },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Schedule {
    pub when: When,
    /// The time zone the plan is written in.
    pub zone: Tz,
}

impl Schedule {
    /// Reads and validates the time of a new plan as written by a client.
    pub(crate) fn parse(time: Option<&PlanTime>, default_zone: Tz, now: Time) -> AppResult<Self> {
        let schedule = Self::read(time, default_zone)?;
        schedule.ensure_in_range(now)?;
        Ok(schedule)
    }

    /// Reads a time as written by a client, checking that it makes sense in
    /// itself (not yet that it is anywhere near the present).
    fn read(time: Option<&PlanTime>, default_zone: Tz) -> AppResult<Self> {
        let Some(time) = time else { return Ok(Self { when: When::Undecided, zone: default_zone }) };
        let zone = input::zone(&time.time_zone, default_zone)?;
        let when = match (&time.start, &time.end) {
            (None, None) => When::Undecided,
            (None, Some(_)) => bail!(invalid, "TIME_INVALID", "an end needs a start"),
            (Some(plan_time::Start::StartAt(start)), end) => {
                let start = from_ts(start);
                let end = match end {
                    None => None,
                    Some(plan_time::End::EndAt(end)) => Some(from_ts(end)),
                    Some(plan_time::End::EndDate(_)) => {
                        bail!(invalid, "TIME_INVALID", "start and end must both be times or both be days")
                    }
                };
                ensure!(
                    end.is_none_or(|end| end > start),
                    invalid,
                    "TIME_INVALID",
                    "the plan must end after it starts"
                );
                ensure!(
                    end.is_none_or(|end| end - start <= Duration::days(MAX_LENGTH_DAYS)),
                    invalid,
                    "TIME_INVALID",
                    "a plan lasts at most {MAX_LENGTH_DAYS} days"
                );
                When::At { start, end }
            }
            (Some(plan_time::Start::StartDate(start)), end) => {
                let start = input::date(start, "TIME_INVALID")?;
                let end = match end {
                    None => None,
                    Some(plan_time::End::EndDate(end)) => Some(input::date(end, "TIME_INVALID")?),
                    Some(plan_time::End::EndAt(_)) => {
                        bail!(invalid, "TIME_INVALID", "start and end must both be times or both be days")
                    }
                };
                ensure!(
                    end.is_none_or(|end| end >= start),
                    invalid,
                    "TIME_INVALID",
                    "the plan cannot end before it starts"
                );
                ensure!(
                    end.is_none_or(|end| (end - start).num_days() <= MAX_LENGTH_DAYS),
                    invalid,
                    "TIME_INVALID",
                    "a plan lasts at most {MAX_LENGTH_DAYS} days"
                );
                When::On { start, end }
            }
        };
        Ok(Self { when, zone })
    }

    /// A plan may be written down a while after the fact (to split the
    /// bill) and planned a few years ahead, but no further.
    fn ensure_in_range(&self, now: Time) -> AppResult<()> {
        if let Some(starts_at) = self.starts_at() {
            ensure!(
                starts_at >= now - Duration::days(MAX_PAST_DAYS),
                invalid,
                "TIME_INVALID",
                "the plan lies too far in the past"
            );
            ensure!(
                starts_at <= now + Duration::days(MAX_AHEAD_DAYS),
                invalid,
                "TIME_INVALID",
                "the plan lies too far in the future"
            );
        }
        Ok(())
    }

    /// The instant the plan starts: local midnight for a plan on a day.
    pub(crate) fn starts_at(&self) -> Option<Time> {
        match &self.when {
            When::Undecided => None,
            When::At { start, .. } => Some(*start),
            When::On { start, .. } => Some(dates::day_start(*start, self.zone)),
        }
    }

    /// The instant after which the plan counts as over.
    pub(crate) fn over_at(&self) -> Option<Time> {
        match &self.when {
            When::Undecided => None,
            When::At { start, end } => Some(end.unwrap_or(*start + Duration::hours(DEFAULT_LENGTH_HOURS))),
            When::On { start, end } => Some(dates::day_end(end.unwrap_or(*start), self.zone)),
        }
    }

    pub(crate) fn to_proto(&self) -> PlanTime {
        let (start, end) = match &self.when {
            When::Undecided => (None, None),
            When::At { start, end } => (
                Some(plan_time::Start::StartAt(Box::new((*start).into()))),
                end.map(|end| plan_time::End::EndAt(Box::new(end.into()))),
            ),
            When::On { start, end } => (
                Some(plan_time::Start::StartDate(dates::format_date(*start))),
                end.map(|end| plan_time::End::EndDate(dates::format_date(end))),
            ),
        };
        PlanTime { start, end, time_zone: self.zone.name().to_owned(), ..Default::default() }
    }

    /// "11/24(火) 18:00" / "11/24(火)" / "日時未定", in the plan's time zone.
    pub(crate) fn text(&self) -> (String, String) {
        match &self.when {
            When::Undecided => (copy::TIME_UNDECIDED.0.to_owned(), copy::TIME_UNDECIDED.1.to_owned()),
            When::At { start, .. } => copy::time_text(*start, self.zone),
            When::On { start, .. } => dates::day_text(*start),
        }
    }

    /// When the reminders and the "attended" events of a plan with this
    /// schedule are due, seen from `now`. Anything whose moment has already
    /// passed is not due at all: a plan made an hour before it starts gets
    /// no "tomorrow" reminder, and one written down after the fact counts
    /// nobody as having attended. (A plan whose attendance was announced is
    /// never announced again, whatever its time is changed to: see
    /// `plans.attended_at`.)
    fn due(&self, remind_day_before: bool, remind_minutes_before: i32, day_before_hour: u32, now: Time) -> Due {
        let ahead = |time: Option<Time>| time.filter(|time| *time > now);
        let day_before = match &self.when {
            When::Undecided => None,
            When::At { start, .. } => Some(*start - Duration::days(1)),
            When::On { start, .. } => start.pred_opt().map(|eve| dates::local_instant(eve, day_before_hour, self.zone)),
        };
        let before = match &self.when {
            When::At { start, .. } if remind_minutes_before > 0 => {
                Some(*start - Duration::minutes(remind_minutes_before as i64))
            }
            _ => None,
        };
        Due {
            day_before: ahead(day_before).filter(|_| remind_day_before),
            before: ahead(before),
            attended: self.over_at().filter(|_| self.starts_at().is_some_and(|starts_at| starts_at > now)),
        }
    }
}

/// When the worker has something to do for a plan (see [`Schedule::due`]).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Due {
    day_before: Option<Time>,
    before: Option<Time>,
    attended: Option<Time>,
}

/// Where a plan happens.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Place {
    name: String,
    location: Option<(f64, f64)>,
    place_id: String,
    address: String,
}

impl Place {
    /// Reads a place written by a client; a place without a name is no place.
    pub(crate) fn parse(place: Option<&PlanPlace>) -> AppResult<Option<Self>> {
        let Some(place) = place else { return Ok(None) };
        let name = input::line(&place.name, MAX_PLACE_NAME, "PLACE_INVALID")?;
        if name.is_empty() {
            return Ok(None);
        }
        let place_id = place.place_id.trim();
        ensure!(
            place_id.len() <= MAX_PLACE_ID && !place_id.chars().any(char::is_control),
            invalid,
            "PLACE_INVALID",
            "the place ID is not valid"
        );
        Ok(Some(Self {
            name,
            location: input::coordinates(place.location.as_option(), "PLACE_INVALID")?,
            place_id: place_id.to_owned(),
            address: input::line(&place.address, MAX_PLACE_ADDRESS, "PLACE_INVALID")?,
        }))
    }

    fn of(row: &PlanRow) -> Option<Self> {
        (!row.place_name.is_empty()).then(|| Self {
            name: row.place_name.clone(),
            location: row.place_lat.zip(row.place_lng),
            place_id: row.place_id.clone(),
            address: row.place_address.clone(),
        })
    }
}

/// Everything needed to store a new plan.
pub(crate) struct NewPlan {
    pub title: String,
    pub emoji: String,
    pub description: String,
    pub schedule: Schedule,
    pub place: Option<Place>,
    pub guests_can_invite: bool,
    pub capacity: i32,
    pub remind_day_before: bool,
    pub remind_minutes_before: i32,
    pub currency: String,
    pub poll_id: Option<Uuid>,
}

pub(crate) fn plan_title(input: &str) -> AppResult<String> {
    input::required_line(input, MAX_TITLE, "TITLE_REQUIRED", "TITLE_TOO_LONG")
}

pub(crate) fn plan_description(input: &str) -> AppResult<String> {
    input::paragraph(input, MAX_DESCRIPTION, "DESCRIPTION_TOO_LONG")
}

fn capacity(value: i32) -> AppResult<i32> {
    ensure!(
        (0..=MAX_CAPACITY).contains(&value),
        invalid,
        "CAPACITY_INVALID",
        "capacity is 0 (no limit) or a number of people up to {MAX_CAPACITY}"
    );
    Ok(value)
}

fn remind_minutes(value: i32) -> AppResult<i32> {
    ensure!(
        (0..=MAX_REMIND_MINUTES).contains(&value),
        invalid,
        "REMINDER_INVALID",
        "the reminder is 0 (off) to {MAX_REMIND_MINUTES} minutes before the start"
    );
    Ok(value)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ParticipantRow {
    pub plan_id: Uuid,
    pub user_id: Uuid,
    pub role: i16,
    pub rsvp: i16,
    pub comment: String,
    pub invited_by: Option<Uuid>,
    pub responded_at: Option<Time>,
}

macro_rules! participant_columns {
    () => {
        "plan_id, user_id, role, rsvp, comment, invited_by, responded_at"
    };
}
pub(crate) use participant_columns;

impl ParticipantRow {
    pub(crate) fn role(&self) -> PlanRole {
        PlanRole::from_i32(self.role as i32).unwrap_or(PlanRole::PLAN_ROLE_UNSPECIFIED)
    }

    pub(crate) fn rsvp(&self) -> Rsvp {
        Rsvp::from_i32(self.rsvp as i32).unwrap_or(Rsvp::RSVP_UNSPECIFIED)
    }

    pub(crate) fn is_organizer(&self) -> bool {
        matches!(self.role(), PlanRole::PLAN_ROLE_HOST | PlanRole::PLAN_ROLE_CO_HOST)
    }
}

/// How many people gave which answer.
pub(crate) fn counts_of<'a>(participants: impl IntoIterator<Item = &'a ParticipantRow>) -> PlanCounts {
    let mut counts = PlanCounts::default();
    for participant in participants {
        match participant.rsvp() {
            Rsvp::RSVP_GOING => counts.going += 1,
            Rsvp::RSVP_MAYBE => counts.maybe += 1,
            Rsvp::RSVP_DECLINED => counts.declined += 1,
            Rsvp::RSVP_INVITED => counts.invited += 1,
            Rsvp::RSVP_WAITLISTED => counts.waitlisted += 1,
            Rsvp::RSVP_UNSPECIFIED => {}
        }
    }
    counts
}

/// A plan together with the caller's place in it: the answer to "may they?".
pub(crate) struct Access {
    pub plan: PlanRow,
    pub me: ParticipantRow,
}

impl Access {
    pub(crate) fn is_host(&self) -> bool {
        self.me.role() == PlanRole::PLAN_ROLE_HOST
    }

    /// Host or co-host.
    pub(crate) fn is_organizer(&self) -> bool {
        self.me.is_organizer()
    }

    /// "Anyone going": the organizers and the guests who answered GOING.
    pub(crate) fn is_going(&self) -> bool {
        self.is_organizer() || self.me.rsvp() == Rsvp::RSVP_GOING
    }

    pub(crate) fn ensure_active(&self) -> AppResult<()> {
        ensure!(self.plan.is_active(), precondition, "PLAN_CANCELLED", "the plan was cancelled");
        Ok(())
    }

    pub(crate) fn ensure_host(&self) -> AppResult<()> {
        ensure!(self.is_host(), denied, "NOT_THE_HOST", "only the host can do this");
        Ok(())
    }

    pub(crate) fn ensure_organizer(&self) -> AppResult<()> {
        ensure!(self.is_organizer(), denied, "NOT_A_HOST", "only the host and co-hosts can do this");
        Ok(())
    }

    pub(crate) fn ensure_going(&self) -> AppResult<()> {
        ensure!(self.is_going(), denied, "NOT_GOING", "only people who are going can do this");
        Ok(())
    }
}

/// Loads a plan as far as `user_id` is in it. `lock` takes the plan's row
/// lock for the rest of the transaction, which serializes everything that
/// changes who is in the plan (invitations, answers, the waitlist).
pub(crate) async fn load_access(
    conn: &mut PgConnection,
    plan_id: Uuid,
    user_id: Uuid,
    lock: bool,
) -> AppResult<Access> {
    let sql = if lock {
        concat!("SELECT ", plan_columns!(), " FROM plans p WHERE p.id = $1 AND p.removed_at IS NULL FOR UPDATE")
    } else {
        concat!("SELECT ", plan_columns!(), " FROM plans p WHERE p.id = $1 AND p.removed_at IS NULL")
    };
    let plan: Option<PlanRow> = sqlx::query_as(sql).bind(plan_id).fetch_optional(&mut *conn).await?;
    let me: Option<ParticipantRow> = sqlx::query_as(concat!(
        "SELECT ",
        participant_columns!(),
        " FROM plan_participants WHERE plan_id = $1 AND user_id = $2"
    ))
    .bind(plan_id)
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    match (plan, me) {
        (Some(plan), Some(me)) => Ok(Access { plan, me }),
        // Someone who is not in the plan may not learn that it exists.
        _ => Err(AppError::not_found("PLAN_NOT_FOUND", "plan not found")),
    }
}

/// Who is told when `actor` changes or cancels a plan: everyone who has not
/// declined, except the actor and the people in `hidden` — those in a block
/// with the actor, in either direction: nothing one of the two does reaches
/// the other. (`hidden` is looked up before the transaction starts.)
async fn participants_to_tell(
    conn: &mut PgConnection,
    plan_id: Uuid,
    actor: Uuid,
    hidden: &[Uuid],
) -> AppResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar("SELECT user_id FROM plan_participants WHERE plan_id = $1 AND rsvp <> $2 AND user_id <> $3 AND NOT (user_id = ANY($4))")
        .bind(plan_id)
        .bind(Rsvp::RSVP_DECLINED as i32 as i16)
        .bind(actor)
        .bind(hidden)
        .fetch_all(conn)
        .await?)
}

/// Everyone in a plan.
pub(crate) async fn participant_ids(conn: &mut PgConnection, plan_id: Uuid) -> AppResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar("SELECT user_id FROM plan_participants WHERE plan_id = $1")
        .bind(plan_id)
        .fetch_all(conn)
        .await?)
}

/// How much of a plan to build.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Detail {
    /// Every participant and the checklist.
    Full,
    /// For lists: the host and the first few people going.
    Preview,
}

impl Plans {
    /// The plan `plan_id` and `user_id`'s place in it (`PLAN_NOT_FOUND` when
    /// they are not in it).
    pub(crate) async fn plan_access(&self, plan_id: Uuid, user_id: Uuid) -> AppResult<Access> {
        let mut conn = self.core.db.acquire().await?;
        load_access(&mut conn, plan_id, user_id, false).await
    }

    /// One plan, in full, as `viewer` (who is in it) sees it.
    pub(crate) async fn plan_for(&self, row: PlanRow, viewer: Uuid) -> AppResult<Plan> {
        Ok(self.plans_to_proto(vec![row], viewer, Detail::Full).await?.remove(0))
    }

    /// `plan_id` in full, as `viewer` sees it.
    pub(crate) async fn get_plan(&self, plan_id: Uuid, viewer: Uuid) -> AppResult<Plan> {
        let access = self.plan_access(plan_id, viewer).await?;
        self.plan_for(access.plan, viewer).await
    }

    /// Builds the API form of plans `viewer` is in.
    pub(crate) async fn plans_to_proto(
        &self,
        rows: Vec<PlanRow>,
        viewer: Uuid,
        detail: Detail,
    ) -> AppResult<Vec<Plan>> {
        let core = &self.core;
        let plan_ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();

        // Organizers first, then by answer (going, maybe, waiting, invited,
        // declined); the waitlist in queue order.
        let listed: Vec<ParticipantRow> = sqlx::query_as(concat!(
            "SELECT ",
            participant_columns!(),
            " FROM plan_participants WHERE plan_id = ANY($1)
              ORDER BY plan_id, role, CASE rsvp WHEN 2 THEN 0 WHEN 3 THEN 1 WHEN 5 THEN 2 WHEN 1 THEN 3 ELSE 4 END,
                       COALESCE(waitlisted_at, created_at), user_id"
        ))
        .bind(&plan_ids)
        .fetch_all(&core.db)
        .await?;
        let mut participants: HashMap<Uuid, Vec<ParticipantRow>> = HashMap::new();
        for participant in listed {
            participants.entry(participant.plan_id).or_default().push(participant);
        }

        let mut checklists: HashMap<Uuid, Vec<ChecklistRow>> = HashMap::new();
        if detail == Detail::Full {
            for item in crate::checklist::items_of(core, &plan_ids).await? {
                checklists.entry(item.plan_id).or_default().push(item);
            }
        }
        let checklist_counts: HashMap<Uuid, (i64, i64)> =
            sqlx::query_as::<_, (Uuid, i64, i64)>("SELECT plan_id, count(*), count(*) FILTER (WHERE done) FROM plan_checklist_items WHERE plan_id = ANY($1) GROUP BY plan_id")
                .bind(&plan_ids)
                .fetch_all(&core.db)
                .await?
                .into_iter()
                .map(|(plan_id, total, done)| (plan_id, (total, done)))
                .collect();
        let expense_counts: HashMap<Uuid, i64> = sqlx::query_as::<_, (Uuid, i64)>(
            "SELECT plan_id, count(*) FROM plan_expenses WHERE plan_id = ANY($1) GROUP BY plan_id",
        )
        .bind(&plan_ids)
        .fetch_all(&core.db)
        .await?
        .into_iter()
        .collect();

        let shown = |all: &[ParticipantRow]| -> Vec<ParticipantRow> {
            match detail {
                Detail::Full => all.to_vec(),
                Detail::Preview => {
                    let host = all.iter().filter(|p| p.role() == PlanRole::PLAN_ROLE_HOST);
                    let going = all
                        .iter()
                        .filter(|p| p.role() != PlanRole::PLAN_ROLE_HOST && p.rsvp() == Rsvp::RSVP_GOING)
                        .take(PREVIEW_PARTICIPANTS);
                    host.chain(going).cloned().collect()
                }
            }
        };
        let needed: HashSet<Uuid> = participants.values().flat_map(|all| shown(all)).map(|p| p.user_id).collect();
        // A block between two participants does not show here: both are
        // listed like everyone else (blocking is silent).
        let needed: Vec<Uuid> = needed.into_iter().collect();
        let people = tokuly_identity::summaries(core, &needed).await?;

        let now = now();
        Ok(rows
            .into_iter()
            .map(|row| {
                let all = participants.remove(&row.id).unwrap_or_default();
                let me = all.iter().find(|p| p.user_id == viewer);
                let mut waiting = 0;
                let listed = shown(&all)
                    .into_iter()
                    .map(|p| {
                        let rsvp = p.rsvp();
                        if rsvp == Rsvp::RSVP_WAITLISTED {
                            waiting += 1;
                        }
                        PlanParticipant {
                            user: users::pick(&people, p.user_id).into(),
                            role: p.role().into(),
                            rsvp: rsvp.into(),
                            comment: p.comment,
                            responded_at: opt_ts(p.responded_at),
                            invited_by_user_id: p.invited_by.map(|id| id.to_string()).unwrap_or_default(),
                            waitlist_position: if rsvp == Rsvp::RSVP_WAITLISTED { waiting } else { 0 },
                            ..Default::default()
                        }
                    })
                    .collect();
                let (checklist_count, checklist_done_count) =
                    checklist_counts.get(&row.id).copied().unwrap_or_default();
                Plan {
                    id: row.id.to_string(),
                    title: row.title.clone(),
                    emoji: row.emoji.clone(),
                    description: row.description.clone(),
                    time: row.schedule().to_proto().into(),
                    place: row.place().into(),
                    status: PlanStatus::from_i32(row.status as i32)
                        .unwrap_or(PlanStatus::PLAN_STATUS_UNSPECIFIED)
                        .into(),
                    host_user_id: row.host_id.to_string(),
                    guests_can_invite: row.guests_can_invite,
                    capacity: row.capacity,
                    remind_day_before: row.remind_day_before,
                    remind_minutes_before: row.remind_minutes_before,
                    currency: row.currency.clone(),
                    counts: counts_of(&all).into(),
                    participants: listed,
                    my_role: me.map(ParticipantRow::role).unwrap_or(PlanRole::PLAN_ROLE_UNSPECIFIED).into(),
                    my_rsvp: me.map(ParticipantRow::rsvp).unwrap_or(Rsvp::RSVP_UNSPECIFIED).into(),
                    checklist: checklists
                        .remove(&row.id)
                        .unwrap_or_default()
                        .iter()
                        .map(ChecklistRow::to_proto)
                        .collect(),
                    checklist_count: checklist_count as i32,
                    checklist_done_count: checklist_done_count as i32,
                    expense_count: expense_counts.get(&row.id).copied().unwrap_or(0) as i32,
                    comment_count: row.comment_count,
                    poll_id: row.poll_id.map(|id| id.to_string()).unwrap_or_default(),
                    cancel_reason: row.cancel_reason.clone(),
                    cancelled_at: opt_ts(row.cancelled_at),
                    over: row.is_over(now),
                    created_at: ts(row.created_at),
                    updated_at: ts(row.updated_at),
                    ..Default::default()
                }
            })
            .collect())
    }

    /// Creates a plan hosted by `user_id` and invites friends to it.
    pub(crate) async fn create_plan(&self, user_id: Uuid, req: &CreatePlanRequest) -> AppResult<Plan> {
        let core = &self.core;
        core.cache.rate_limit(&format!("plans:create:{user_id}"), 30, StdDuration::from_secs(3600)).await?;
        let zone = dates::zone_of(core, user_id).await?;
        let new = NewPlan {
            title: plan_title(&req.title)?,
            emoji: input::emoji(&req.emoji)?,
            description: plan_description(&req.description)?,
            schedule: Schedule::parse(req.time.as_option(), zone, now())?,
            place: Place::parse(req.place.as_option())?,
            guests_can_invite: req.guests_can_invite,
            capacity: capacity(req.capacity)?,
            remind_day_before: req.remind_day_before.unwrap_or(true),
            remind_minutes_before: remind_minutes(req.remind_minutes_before.unwrap_or(DEFAULT_REMIND_MINUTES))?,
            currency: input::currency(&req.currency, DEFAULT_CURRENCY)?,
            poll_id: None,
        };
        let invitees = input::people(&req.invitee_ids, user_id)?;
        ensure!(
            (invitees.len() as i64) < self.cfg.max_participants,
            invalid,
            "TOO_MANY_PARTICIPANTS",
            "a plan holds at most {} people",
            self.cfg.max_participants
        );
        people::require_friends(core, user_id, &invitees).await?;
        people::limit_invitations(core, user_id, &invitees).await?;

        let mut tx = core.db.begin().await?;
        let row = self.insert_plan(&mut tx, user_id, &new, &invitees).await?;
        tx.commit().await?;

        self.push(invitees, plan_changed(row.id, PlanChange::PLAN_CHANGE_INVITED)).await;
        self.plan_for(row, user_id).await
    }

    /// Stores a new plan with its host and invitees, announces it
    /// (`plans.plan.created`, an entry in the host's activity log) and
    /// invites the invitees. Shared by `CreatePlan` and by date polls that
    /// turn into a plan.
    pub(crate) async fn insert_plan(
        &self,
        conn: &mut PgConnection,
        host: Uuid,
        new: &NewPlan,
        invitees: &[Uuid],
    ) -> AppResult<PlanRow> {
        let due = new.schedule.due(new.remind_day_before, new.remind_minutes_before, self.cfg.day_before_hour, now());
        let (start_at, start_date, end_at, end_date) = match &new.schedule.when {
            When::Undecided => (None, None, None, None),
            When::At { start, end } => (Some(*start), None, *end, None),
            When::On { start, end } => (None, Some(*start), None, *end),
        };
        let start_kind: i16 = match &new.schedule.when {
            When::Undecided => 0,
            When::At { .. } => 1,
            When::On { .. } => 2,
        };
        let row: PlanRow = sqlx::query_as(concat!(
            "INSERT INTO plans AS p (host_id, title, emoji, description, start_kind, start_at, start_date, end_at, end_date, time_zone,
                                     sort_at, over_at, place_name, place_lat, place_lng, place_id, place_address, guests_can_invite, capacity,
                                     remind_day_before, remind_minutes_before, day_before_due_at, before_due_at, attended_due_at, currency, poll_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26)
             RETURNING ",
            plan_columns!()
        ))
        .bind(host)
        .bind(&new.title)
        .bind(&new.emoji)
        .bind(&new.description)
        .bind(start_kind)
        .bind(start_at)
        .bind(start_date)
        .bind(end_at)
        .bind(end_date)
        .bind(new.schedule.zone.name())
        .bind(new.schedule.starts_at())
        .bind(new.schedule.over_at())
        .bind(new.place.as_ref().map(|place| place.name.as_str()).unwrap_or_default())
        .bind(new.place.as_ref().and_then(|place| place.location).map(|(lat, _)| lat))
        .bind(new.place.as_ref().and_then(|place| place.location).map(|(_, lng)| lng))
        .bind(new.place.as_ref().map(|place| place.place_id.as_str()).unwrap_or_default())
        .bind(new.place.as_ref().map(|place| place.address.as_str()).unwrap_or_default())
        .bind(new.guests_can_invite)
        .bind(new.capacity)
        .bind(new.remind_day_before)
        .bind(new.remind_minutes_before)
        .bind(due.day_before)
        .bind(due.before)
        .bind(due.attended)
        .bind(&new.currency)
        .bind(new.poll_id)
        .fetch_one(&mut *conn)
        .await?;

        // The host is in their own plan, and going.
        sqlx::query(
            "INSERT INTO plan_participants (plan_id, user_id, role, rsvp, responded_at) VALUES ($1, $2, $3, $4, now())",
        )
        .bind(row.id)
        .bind(host)
        .bind(PlanRole::PLAN_ROLE_HOST as i32 as i16)
        .bind(Rsvp::RSVP_GOING as i32 as i16)
        .execute(&mut *conn)
        .await?;
        self.invite(conn, &row, host, invitees).await?;

        let created = PlanCreated {
            plan_id: row.id,
            host_id: host,
            invitee_ids: invitees.to_vec(),
            starts_at: row.sort_at,
            poll_id: row.poll_id,
        };
        bus::emit(&mut *conn, PLAN_CREATED, Some(host), &created).await?;
        // Only the host sees the entry (in their own log): a plan is nobody else's business.
        let mut entry = ActivityEntry::new(
            host,
            "plans.created",
            ActivityAudience::Private,
            copy::plan_created_summary(&row.name()),
        )
        .subject(SubjectType::SUBJECT_TYPE_PLAN, row.id)
        .with(invitees.iter().copied().take(20))
        .path(plan_path(row.id));
        if let Some((lat, lng)) = row.place_lat.zip(row.place_lng) {
            entry = entry.at(lat, lng);
        }
        outbound::activity(&mut *conn, &entry).await?;
        Ok(row)
    }

    /// Changes a plan (organizers only).
    pub(crate) async fn update_plan(&self, user_id: Uuid, req: &UpdatePlanRequest) -> AppResult<Plan> {
        let core = &self.core;
        let plan_id = parse_id(&req.plan_id)?;
        let now = now();
        // A change of time or place notifies everyone in the plan.
        core.cache
            .rate_limit(&format!("plans:update:{user_id}"), CHANGES_PER_HOUR, StdDuration::from_secs(3600))
            .await?;
        // Asked before the plan is locked: inside the transaction nothing goes through the pool.
        let hidden = graph::blocked_ids_either(core, user_id).await?;

        let mut tx = core.db.begin().await?;
        let access = load_access(&mut tx, plan_id, user_id, true).await?;
        access.ensure_organizer()?;
        access.ensure_active()?;
        let old = &access.plan;

        let title = req.title.as_deref().map(plan_title).transpose()?.unwrap_or_else(|| old.title.clone());
        let emoji = req.emoji.as_deref().map(input::emoji).transpose()?.unwrap_or_else(|| old.emoji.clone());
        let description =
            req.description.as_deref().map(plan_description).transpose()?.unwrap_or_else(|| old.description.clone());
        let old_schedule = old.schedule();
        let schedule = match req.time.as_option() {
            // A plan keeps its time zone unless the request names another one.
            // A time that stays the same is not checked against the present
            // again: an old plan can still be edited.
            Some(time) => {
                let schedule = Schedule::read(Some(time), old_schedule.zone)?;
                if schedule != old_schedule {
                    schedule.ensure_in_range(now)?;
                }
                schedule
            }
            None => old_schedule.clone(),
        };
        let old_place = Place::of(old);
        let place = if req.place.is_set() { Place::parse(req.place.as_option())? } else { old_place.clone() };
        let guests_can_invite = req.guests_can_invite.unwrap_or(old.guests_can_invite);
        let capacity = req.capacity.map(capacity).transpose()?.unwrap_or(old.capacity);
        let remind_day_before = req.remind_day_before.unwrap_or(old.remind_day_before);
        let remind_minutes_before =
            req.remind_minutes_before.map(remind_minutes).transpose()?.unwrap_or(old.remind_minutes_before);
        let currency = req
            .currency
            .as_deref()
            .map(|code| input::currency(code, DEFAULT_CURRENCY))
            .transpose()?
            .unwrap_or_else(|| old.currency.clone());

        let time_changed = schedule != old_schedule;
        let place_changed = place != old_place;
        if time_changed || place_changed {
            // (Valkey, not the pool: nothing here waits for a database connection.)
            core.cache
                .rate_limit(
                    &format!("plans:update:announced:{plan_id}"),
                    ANNOUNCED_CHANGES_PER_HOUR,
                    StdDuration::from_secs(3600),
                )
                .await?;
        }
        let reminders_changed = time_changed
            || remind_day_before != old.remind_day_before
            || remind_minutes_before != old.remind_minutes_before;
        let due = schedule.due(remind_day_before, remind_minutes_before, self.cfg.day_before_hour, now);
        let (start_at, start_date, end_at, end_date) = match &schedule.when {
            When::Undecided => (None, None, None, None),
            When::At { start, end } => (Some(*start), None, *end, None),
            When::On { start, end } => (None, Some(*start), None, *end),
        };
        let start_kind: i16 = match &schedule.when {
            When::Undecided => 0,
            When::At { .. } => 1,
            When::On { .. } => 2,
        };
        let row: PlanRow = sqlx::query_as(concat!(
            "UPDATE plans p SET
                 title = $2, emoji = $3, description = $4, start_kind = $5, start_at = $6, start_date = $7, end_at = $8, end_date = $9,
                 time_zone = $10, sort_at = $11, over_at = $12, place_name = $13, place_lat = $14, place_lng = $15, place_id = $16,
                 place_address = $17, guests_can_invite = $18, capacity = $19, remind_day_before = $20, remind_minutes_before = $21,
                 currency = $22,
                 day_before_due_at = CASE WHEN $23 THEN $24 ELSE day_before_due_at END,
                 before_due_at = CASE WHEN $23 THEN $25 ELSE before_due_at END,
                 attended_due_at = CASE WHEN $26 AND attended_at IS NULL THEN $27 ELSE attended_due_at END
             WHERE p.id = $1 RETURNING ",
            plan_columns!()
        ))
        .bind(plan_id)
        .bind(&title)
        .bind(&emoji)
        .bind(&description)
        .bind(start_kind)
        .bind(start_at)
        .bind(start_date)
        .bind(end_at)
        .bind(end_date)
        .bind(schedule.zone.name())
        .bind(schedule.starts_at())
        .bind(schedule.over_at())
        .bind(place.as_ref().map(|place| place.name.as_str()).unwrap_or_default())
        .bind(place.as_ref().and_then(|place| place.location).map(|(lat, _)| lat))
        .bind(place.as_ref().and_then(|place| place.location).map(|(_, lng)| lng))
        .bind(place.as_ref().map(|place| place.place_id.as_str()).unwrap_or_default())
        .bind(place.as_ref().map(|place| place.address.as_str()).unwrap_or_default())
        .bind(guests_can_invite)
        .bind(capacity)
        .bind(remind_day_before)
        .bind(remind_minutes_before)
        .bind(&currency)
        .bind(reminders_changed)
        .bind(due.day_before)
        .bind(due.before)
        .bind(time_changed)
        .bind(due.attended)
        .fetch_one(&mut *tx)
        .await?;

        // More room lets people on the waitlist in.
        let promoted =
            if capacity != old.capacity { self.promote_waitlisted(&mut tx, &row).await? } else { Vec::new() };

        if time_changed || place_changed {
            let text = copy::plan_updated(
                &row.name(),
                time_changed,
                &schedule.text(),
                place.as_ref().filter(|_| place_changed).map(|place| place.name.as_str()),
            );
            for participant in participants_to_tell(&mut tx, plan_id, user_id, &hidden).await? {
                let request = NotificationRequest::new(
                    participant,
                    "plans.updated",
                    text.title.clone(),
                    text.body.clone(),
                    plan_path(plan_id),
                )
                .actor(user_id)
                .subject(SubjectType::SUBJECT_TYPE_PLAN, plan_id)
                .collapse(format!("plans.updated:{plan_id}"));
                outbound::notify(&mut *tx, &request).await?;
            }
        }
        let everyone = participant_ids(&mut tx, plan_id).await?;
        tx.commit().await?;

        self.push(everyone, plan_changed(plan_id, PlanChange::PLAN_CHANGE_UPDATED)).await;
        self.push_rsvps(plan_id, &promoted).await?;
        self.plan_for(row, user_id).await
    }

    /// Calls a plan off (host only). It stays readable for everyone in it.
    pub(crate) async fn cancel_plan(&self, user_id: Uuid, plan_id: Uuid, reason: &str) -> AppResult<Plan> {
        let core = &self.core;
        let reason = input::line(reason, MAX_CANCEL_REASON, "REASON_TOO_LONG")?;
        core.cache
            .rate_limit(&format!("plans:update:{user_id}"), CHANGES_PER_HOUR, StdDuration::from_secs(3600))
            .await?;
        let hidden = graph::blocked_ids_either(core, user_id).await?;

        let mut tx = core.db.begin().await?;
        let access = load_access(&mut tx, plan_id, user_id, true).await?;
        access.ensure_host()?;
        access.ensure_active()?;
        let row: PlanRow = sqlx::query_as(concat!(
            "UPDATE plans p SET status = $2, cancelled_at = now(), cancel_reason = $3,
                    day_before_due_at = NULL, before_due_at = NULL, attended_due_at = NULL
             WHERE p.id = $1 RETURNING ",
            plan_columns!()
        ))
        .bind(plan_id)
        .bind(PlanStatus::PLAN_STATUS_CANCELLED as i32 as i16)
        .bind(&reason)
        .fetch_one(&mut *tx)
        .await?;
        bus::emit(&mut *tx, PLAN_CANCELLED, Some(user_id), &PlanCancelled { plan_id, host_id: user_id }).await?;

        let text = copy::plan_cancelled(&row.name(), &reason);
        for participant in participants_to_tell(&mut tx, plan_id, user_id, &hidden).await? {
            let request = NotificationRequest::new(
                participant,
                "plans.cancelled",
                text.title.clone(),
                text.body.clone(),
                plan_path(plan_id),
            )
            .actor(user_id)
            .subject(SubjectType::SUBJECT_TYPE_PLAN, plan_id)
            .collapse(format!("plans.cancelled:{plan_id}"));
            outbound::notify(&mut *tx, &request).await?;
        }
        let everyone = participant_ids(&mut tx, plan_id).await?;
        tx.commit().await?;

        self.push(everyone, plan_changed(plan_id, PlanChange::PLAN_CHANGE_CANCELLED)).await;
        self.plan_for(row, user_id).await
    }

    /// `user_id`'s plans, by `filter`.
    pub(crate) async fn list_plans(
        &self,
        user_id: Uuid,
        filter: PlanListFilter,
        include_declined: bool,
        page: &Page,
    ) -> AppResult<(Vec<Plan>, PageInfo)> {
        let past = filter == PlanListFilter::PLAN_LIST_FILTER_PAST;
        let invited_only = filter == PlanListFilter::PLAN_LIST_FILTER_INVITED;
        let cursor: Option<(Time, Uuid)> = page.decode()?;
        let mut rows: Vec<PlanRow> = if past {
            // Over or cancelled, most recent first.
            sqlx::query_as(concat!(
                "SELECT ",
                plan_columns!(),
                " FROM plans p JOIN plan_participants me ON me.plan_id = p.id AND me.user_id = $1
                  WHERE p.removed_at IS NULL AND (p.status = 2 OR p.over_at <= now()) AND ($2 OR me.rsvp <> 4)
                    AND ($3::timestamptz IS NULL OR (COALESCE(p.sort_at, p.cancelled_at, p.created_at), p.id) < ($3, $4))
                  ORDER BY COALESCE(p.sort_at, p.cancelled_at, p.created_at) DESC, p.id DESC
                  LIMIT $5"
            ))
            .bind(user_id)
            .bind(include_declined)
            .bind(cursor.map(|(key, _)| key))
            .bind(cursor.map(|(_, id)| id))
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?
        } else {
            // Still ahead, soonest first; plans without a time at the end.
            sqlx::query_as(concat!(
                "SELECT ",
                plan_columns!(),
                " FROM plans p JOIN plan_participants me ON me.plan_id = p.id AND me.user_id = $1
                  WHERE p.removed_at IS NULL AND p.status = 1 AND (p.over_at IS NULL OR p.over_at > now())
                    AND ($2 OR me.rsvp <> 4) AND (NOT $3 OR me.rsvp = 1)
                    AND ($4::timestamptz IS NULL OR (COALESCE(p.sort_at, $6), p.id) > ($4, $5))
                  ORDER BY COALESCE(p.sort_at, $6), p.id
                  LIMIT $7"
            ))
            .bind(user_id)
            .bind(include_declined)
            .bind(invited_only)
            .bind(cursor.map(|(key, _)| key))
            .bind(cursor.map(|(_, id)| id))
            .bind(far_future())
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?
        };
        let info = page.finish(&mut rows, |row| {
            let key = if past {
                row.sort_at.or(row.cancelled_at).unwrap_or(row.created_at)
            } else {
                row.sort_at.unwrap_or_else(far_future)
            };
            (key, row.id)
        });
        Ok((self.plans_to_proto(rows, user_id, Detail::Preview).await?, info))
    }
}

/// The realtime event saying that a plan changed in some way.
pub(crate) fn plan_changed(plan_id: Uuid, change: PlanChange) -> Push {
    Push::PlanChanged(Box::new(PlanChanged {
        plan_id: plan_id.to_string(),
        change: change.into(),
        ..Default::default()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKYO: Tz = chrono_tz::Asia::Tokyo;

    fn tokyo(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> Time {
        TOKYO.with_ymd_and_hms(year, month, day, hour, minute, 0).unwrap().with_timezone(&Utc)
    }

    fn date(text: &str) -> NaiveDate {
        dates::parse_date(text).unwrap()
    }

    fn at(start: Time, end: Option<Time>) -> PlanTime {
        PlanTime {
            start: Some(plan_time::Start::StartAt(Box::new(start.into()))),
            end: end.map(|end| plan_time::End::EndAt(Box::new(end.into()))),
            ..Default::default()
        }
    }

    fn on(start: &str, end: Option<&str>) -> PlanTime {
        PlanTime {
            start: Some(plan_time::Start::StartDate(start.to_owned())),
            end: end.map(|end| plan_time::End::EndDate(end.to_owned())),
            ..Default::default()
        }
    }

    #[test]
    fn a_time_a_day_or_nothing_yet() {
        let now = tokyo(2026, 11, 22, 12, 0);
        let start = tokyo(2026, 11, 24, 18, 0);

        let timed = Schedule::parse(Some(&at(start, None)), TOKYO, now).unwrap();
        assert_eq!(timed.when, When::At { start, end: None });
        assert_eq!(timed.starts_at(), Some(start));
        // Without an end a plan is over three hours after it starts.
        assert_eq!(timed.over_at(), Some(tokyo(2026, 11, 24, 21, 0)));
        assert_eq!(timed.text().0, "11/24(火) 18:00");

        let day = Schedule::parse(Some(&on("2026-11-24", Some("2026-11-25"))), TOKYO, now).unwrap();
        assert_eq!(day.when, When::On { start: date("2026-11-24"), end: Some(date("2026-11-25")) });
        // A day begins and ends at local midnight.
        assert_eq!(day.starts_at(), Some(tokyo(2026, 11, 24, 0, 0)));
        assert_eq!(day.over_at(), Some(tokyo(2026, 11, 26, 0, 0)));
        assert_eq!(day.text().0, "11/24(火)");

        for undecided in [
            Schedule::parse(None, TOKYO, now).unwrap(),
            Schedule::parse(Some(&PlanTime::default()), TOKYO, now).unwrap(),
        ] {
            assert_eq!(undecided.when, When::Undecided);
            assert_eq!((undecided.starts_at(), undecided.over_at()), (None, None));
            assert_eq!(undecided.text().0, "日時未定");
        }
    }

    #[test]
    fn times_round_trip_through_the_api_form() {
        let now = tokyo(2026, 11, 22, 12, 0);
        for time in [
            at(tokyo(2026, 11, 24, 18, 0), Some(tokyo(2026, 11, 24, 21, 30))),
            on("2026-12-24", None),
            PlanTime::default(),
        ] {
            let schedule = Schedule::parse(Some(&time), TOKYO, now).unwrap();
            assert_eq!(Schedule::parse(Some(&schedule.to_proto()), chrono_tz::UTC, now).unwrap(), schedule);
        }
        // The zone named in the request wins over the caller's.
        let paris = PlanTime { time_zone: "Europe/Paris".into(), ..on("2026-12-24", None) };
        assert_eq!(Schedule::parse(Some(&paris), TOKYO, now).unwrap().zone, chrono_tz::Europe::Paris);
    }

    #[test]
    fn impossible_times_are_rejected() {
        let now = tokyo(2026, 11, 22, 12, 0);
        let start = tokyo(2026, 11, 24, 18, 0);
        let mixed = PlanTime { end: Some(plan_time::End::EndDate("2026-11-25".into())), ..at(start, None) };
        let end_only = PlanTime { end: Some(plan_time::End::EndDate("2026-11-25".into())), ..Default::default() };
        let rejected = [
            at(start, Some(start)),
            at(start, Some(start - Duration::hours(1))),
            at(start, Some(start + Duration::days(32))),
            at(now - Duration::days(40), None),
            at(now + Duration::days(4 * 366), None),
            on("2026-11-24", Some("2026-11-23")),
            on("2026-13-01", None),
            on("tomorrow", None),
            mixed,
            end_only,
        ];
        for time in rejected {
            assert_eq!(Schedule::parse(Some(&time), TOKYO, now).unwrap_err().reason, "TIME_INVALID", "{time:?}");
        }
        let bad_zone = PlanTime { time_zone: "Mars/Olympus".into(), ..on("2026-11-24", None) };
        assert_eq!(Schedule::parse(Some(&bad_zone), TOKYO, now).unwrap_err().reason, "TIME_ZONE_INVALID");
        // Yesterday's dinner can still be written down, to split the bill.
        assert!(Schedule::parse(Some(&on("2026-11-21", None)), TOKYO, now).is_ok());
    }

    #[test]
    fn reminders_are_due_the_day_before_and_shortly_before() {
        let now = tokyo(2026, 11, 22, 12, 0);
        let start = tokyo(2026, 11, 24, 18, 0);
        let timed = Schedule { when: When::At { start, end: None }, zone: TOKYO };
        let due = timed.due(true, 60, 18, now);
        assert_eq!(
            due,
            Due {
                day_before: Some(tokyo(2026, 11, 23, 18, 0)),
                before: Some(tokyo(2026, 11, 24, 17, 0)),
                attended: Some(tokyo(2026, 11, 24, 21, 0))
            }
        );
        // Switched off.
        assert_eq!(
            timed.due(false, 0, 18, now),
            Due { day_before: None, before: None, attended: Some(tokyo(2026, 11, 24, 21, 0)) }
        );

        // A plan on a day is announced the evening before, at the configured hour; it has no "minutes before".
        let day = Schedule { when: When::On { start: date("2026-11-24"), end: None }, zone: TOKYO };
        assert_eq!(
            day.due(true, 60, 18, now),
            Due {
                day_before: Some(tokyo(2026, 11, 23, 18, 0)),
                before: None,
                attended: Some(tokyo(2026, 11, 25, 0, 0))
            }
        );

        let undecided = Schedule { when: When::Undecided, zone: TOKYO };
        assert_eq!(undecided.due(true, 60, 18, now), Due { day_before: None, before: None, attended: None });
    }

    #[test]
    fn what_has_already_passed_is_not_due() {
        // Made at noon for 12:30 today: too late for "tomorrow", in time for "soon" only if it is still ahead.
        let now = tokyo(2026, 11, 22, 12, 0);
        let soon = Schedule { when: When::At { start: tokyo(2026, 11, 22, 12, 30), end: None }, zone: TOKYO };
        assert_eq!(
            soon.due(true, 60, 18, now),
            Due { day_before: None, before: None, attended: Some(tokyo(2026, 11, 22, 15, 30)) }
        );
        assert_eq!(soon.due(true, 15, 18, now).before, Some(tokyo(2026, 11, 22, 12, 15)));
        // Written down after it started: nobody "attended" it as far as rewards go.
        let earlier = Schedule { when: When::At { start: tokyo(2026, 11, 22, 10, 0), end: None }, zone: TOKYO };
        assert_eq!(earlier.due(true, 60, 18, now), Due { day_before: None, before: None, attended: None });
    }

    #[test]
    fn a_place_needs_a_name() {
        let named = PlanPlace {
            name: " 駅前のカラオケ ".into(),
            location: LatLng { lat: 35.66, lng: 139.7, ..Default::default() }.into(),
            ..Default::default()
        };
        let place = Place::parse(Some(&named)).unwrap().unwrap();
        assert_eq!((place.name.as_str(), place.location), ("駅前のカラオケ", Some((35.66, 139.7))));
        assert_eq!(Place::parse(None).unwrap(), None);
        assert_eq!(Place::parse(Some(&PlanPlace { address: "somewhere".into(), ..Default::default() })).unwrap(), None);
        let off_earth = PlanPlace {
            name: "月".into(),
            location: LatLng { lat: 123.0, lng: 0.0, ..Default::default() }.into(),
            ..Default::default()
        };
        assert_eq!(Place::parse(Some(&off_earth)).unwrap_err().reason, "PLACE_INVALID");
        assert_eq!(
            Place::parse(Some(&PlanPlace { name: "あ".repeat(81), ..Default::default() })).unwrap_err().reason,
            "PLACE_INVALID"
        );
    }

    #[test]
    fn numbers_have_sensible_ranges() {
        assert_eq!(capacity(0).unwrap(), 0);
        assert_eq!(capacity(-1).unwrap_err().reason, "CAPACITY_INVALID");
        assert_eq!(capacity(MAX_CAPACITY + 1).unwrap_err().reason, "CAPACITY_INVALID");
        assert_eq!(remind_minutes(MAX_REMIND_MINUTES).unwrap(), MAX_REMIND_MINUTES);
        assert_eq!(remind_minutes(-5).unwrap_err().reason, "REMINDER_INVALID");
        assert_eq!(plan_title("  ").unwrap_err().reason, "TITLE_REQUIRED");
        assert_eq!(plan_title(&"あ".repeat(81)).unwrap_err().reason, "TITLE_TOO_LONG");
    }
}
