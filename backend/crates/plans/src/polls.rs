//! Date polls (日程調整): candidate dates, ○ △ × from everyone, a suggested
//! slot, and a plan at the end.
//!
//! A poll is visible to its creator, to the people in it (invited friends,
//! and anyone who answered), and — when it is open to friends — to any
//! friend of the creator who has its link. The creator has a participant row
//! of their own, so they answer like everyone else.
use std::{collections::HashSet, time::Duration as StdDuration};

use buffa::Enumeration as _;
use chrono::{Duration, NaiveDate};
use chrono_tz::Tz;
use sqlx::PgConnection;
use tokuly_core::{
    events as bus,
    ids::{Ref, new_short_id},
    outbound::{self, NotificationRequest},
    prelude::*,
};
use tokuly_proto::{
    common::PageInfo,
    plans::{plans_event::Event as Push, poll_slot, poll_slot_input, *},
};
use tokuly_social::graph;

use crate::{
    Plans, copy, dates,
    events::{POLL_CREATED, POLL_FINALIZED, PollCreated, PollFinalized},
    input,
    people::{self, People, name_of},
    plan::{NewPlan, Place, Schedule, When, plan_changed, plan_description},
};

const MAX_TITLE: usize = 80;
const MAX_DESCRIPTION: usize = 1000;
const MAX_SLOTS: usize = 30;
const MAX_COMMENT: usize = 140;
/// Candidates lie between yesterday and two years from now.
const MAX_SLOT_AHEAD_DAYS: i64 = 2 * 366;
const MAX_SLOT_LENGTH_DAYS: i64 = 31;
const MAX_DEADLINE_DAYS: i64 = 366;
/// People who have not answered are reminded this long before the deadline,
/// or this long after the poll was made when it has none.
const REMIND_BEFORE_DEADLINE_HOURS: i64 = 24;
const REMIND_WITHOUT_DEADLINE_HOURS: i64 = 48;

/// Path of a poll in the app: its link.
fn poll_path(short_id: &str) -> String {
    format!("/plans/polls/{short_id}")
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct PollRow {
    pub id: Uuid,
    pub short_id: String,
    pub creator_id: Uuid,
    pub title: String,
    pub description: String,
    pub time_zone: String,
    pub open_to_friends: bool,
    pub status: i16,
    pub deadline: Option<Time>,
    pub final_slot_id: Option<Uuid>,
    pub plan_id: Option<Uuid>,
    pub created_at: Time,
    pub updated_at: Time,
}

macro_rules! poll_columns {
    () => {
        "q.id, q.short_id, q.creator_id, q.title, q.description, q.time_zone, q.open_to_friends, q.status, q.deadline, q.final_slot_id, q.plan_id, q.created_at, q.updated_at"
    };
}

impl PollRow {
    fn status(&self) -> PollStatus {
        PollStatus::from_i32(self.status as i32).unwrap_or(PollStatus::POLL_STATUS_UNSPECIFIED)
    }

    fn is_open(&self) -> bool {
        self.status() == PollStatus::POLL_STATUS_OPEN
    }

    fn ensure_open(&self) -> AppResult<()> {
        ensure!(self.is_open(), precondition, "POLL_NOT_OPEN", "the poll no longer takes answers");
        Ok(())
    }

    fn ensure_creator(&self, user_id: Uuid) -> AppResult<()> {
        ensure!(self.creator_id == user_id, denied, "NOT_THE_CREATOR", "only the creator of the poll can do this");
        Ok(())
    }
}

/// A candidate: a time (with an optional end) or a whole day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Candidate {
    At { start: Time, end: Option<Time> },
    On(NaiveDate),
}

impl Candidate {
    /// The instant it starts (local midnight for a day): the order of the
    /// slots and the "earliest" tie-break of the suggestion.
    fn sort_at(&self, zone: Tz) -> Time {
        match self {
            Self::At { start, .. } => *start,
            Self::On(date) => dates::day_start(*date, zone),
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct SlotRow {
    id: Uuid,
    poll_id: Uuid,
    start_at: Option<Time>,
    slot_date: Option<NaiveDate>,
    end_at: Option<Time>,
    sort_at: Time,
}

impl SlotRow {
    fn candidate(&self) -> Candidate {
        match (self.start_at, self.slot_date) {
            (Some(start), _) => Candidate::At { start, end: self.end_at },
            (None, date) => Candidate::On(date.unwrap_or_default()),
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct PollParticipantRow {
    poll_id: Uuid,
    user_id: Uuid,
    invited: bool,
    comment: String,
    answered_at: Option<Time>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct AnswerRow {
    poll_id: Uuid,
    slot_id: Uuid,
    user_id: Uuid,
    answer: i16,
}

/// ○ △ × counted for one slot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Tally {
    pub slot_id: Uuid,
    pub sort_at: Time,
    pub yes: i32,
    pub maybe: i32,
    pub no: i32,
}

/// The slot to suggest: most ○, then fewest ×, then the earliest. Nothing is
/// suggested while nobody has answered.
pub(crate) fn best_slot(tallies: &[Tally]) -> Option<Uuid> {
    if tallies.iter().all(|tally| tally.yes + tally.maybe + tally.no == 0) {
        return None;
    }
    tallies
        .iter()
        .min_by(|a, b| {
            b.yes.cmp(&a.yes).then(a.no.cmp(&b.no)).then(a.sort_at.cmp(&b.sort_at)).then(a.slot_id.cmp(&b.slot_id))
        })
        .map(|tally| tally.slot_id)
}

fn tallies_of(slots: &[SlotRow], answers: &[AnswerRow]) -> Vec<Tally> {
    slots
        .iter()
        .map(|slot| {
            let count = |answer: PollAnswer| {
                answers.iter().filter(|given| given.slot_id == slot.id && given.answer == answer as i32 as i16).count()
                    as i32
            };
            Tally {
                slot_id: slot.id,
                sort_at: slot.sort_at,
                yes: count(PollAnswer::POLL_ANSWER_YES),
                maybe: count(PollAnswer::POLL_ANSWER_MAYBE),
                no: count(PollAnswer::POLL_ANSWER_NO),
            }
        })
        .collect()
}

/// Reads candidates written by a client.
fn parse_slots(inputs: &[PollSlotInput], zone: Tz, now: Time) -> AppResult<Vec<Candidate>> {
    let earliest = now - Duration::days(1);
    let latest = now + Duration::days(MAX_SLOT_AHEAD_DAYS);
    let mut seen = HashSet::new();
    let mut candidates = Vec::with_capacity(inputs.len());
    for input in inputs {
        let candidate = match &input.start {
            Some(poll_slot_input::Start::StartAt(start)) => {
                let start = from_ts(start);
                let end = input.end_at.as_option().map(from_ts);
                ensure!(
                    end.is_none_or(|end| end > start && end - start <= Duration::days(MAX_SLOT_LENGTH_DAYS)),
                    invalid,
                    "SLOT_INVALID",
                    "a slot must end after it starts"
                );
                Candidate::At { start, end }
            }
            Some(poll_slot_input::Start::Date(date)) => {
                ensure!(
                    input.end_at.is_unset(),
                    invalid,
                    "SLOT_INVALID",
                    "only a slot with a start time can have an end"
                );
                Candidate::On(input::date(date, "SLOT_INVALID")?)
            }
            None => bail!(invalid, "SLOT_INVALID", "a slot is a day or a time"),
        };
        let starts = candidate.sort_at(zone);
        ensure!(
            starts >= earliest && starts <= latest,
            invalid,
            "SLOT_INVALID",
            "slots lie between now and two years from now"
        );
        ensure!(seen.insert(candidate), invalid, "SLOT_DUPLICATE", "the same slot is listed twice");
        candidates.push(candidate);
    }
    Ok(candidates)
}

fn parse_deadline(deadline: Option<Time>, now: Time) -> AppResult<Option<Time>> {
    ensure!(
        deadline.is_none_or(|deadline| deadline > now && deadline <= now + Duration::days(MAX_DEADLINE_DAYS)),
        invalid,
        "DEADLINE_INVALID",
        "the deadline must lie in the future, at most a year ahead"
    );
    Ok(deadline)
}

/// When the people who have not answered are reminded automatically: a day
/// before the deadline (not at all when that is less than an hour away), or
/// two days after `now` for a poll without a deadline.
pub(crate) fn remind_due(deadline: Option<Time>, now: Time) -> Option<Time> {
    match deadline {
        Some(deadline) => Some(deadline - Duration::hours(REMIND_BEFORE_DEADLINE_HOURS))
            .filter(|due| *due >= now + Duration::hours(1)),
        None => Some(now + Duration::hours(REMIND_WITHOUT_DEADLINE_HOURS)),
    }
}

async fn insert_slots(conn: &mut PgConnection, poll_id: Uuid, candidates: &[Candidate], zone: Tz) -> AppResult<()> {
    for candidate in candidates {
        let (start_at, slot_date, end_at) = match candidate {
            Candidate::At { start, end } => (Some(*start), None, *end),
            Candidate::On(date) => (None, Some(*date), None),
        };
        sqlx::query(
            "INSERT INTO date_poll_slots (poll_id, start_at, slot_date, end_at, sort_at) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(poll_id)
        .bind(start_at)
        .bind(slot_date)
        .bind(end_at)
        .bind(candidate.sort_at(zone))
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

async fn slots_of(conn: &mut PgConnection, poll_id: Uuid) -> AppResult<Vec<SlotRow>> {
    Ok(sqlx::query_as("SELECT id, poll_id, start_at, slot_date, end_at, sort_at FROM date_poll_slots WHERE poll_id = $1 ORDER BY sort_at, id").bind(poll_id).fetch_all(conn).await?)
}

async fn poll_participant_ids(conn: &mut PgConnection, poll_id: Uuid) -> AppResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar("SELECT user_id FROM date_poll_participants WHERE poll_id = $1")
        .bind(poll_id)
        .fetch_all(conn)
        .await?)
}

/// A poll and whether the caller is already in it.
struct PollAccess {
    poll: PollRow,
    joined: bool,
}

impl Plans {
    /// Loads a poll by ID or link as far as `user_id` may see it. `lock`
    /// takes the poll's row lock for the rest of the transaction.
    async fn poll_access(
        &self,
        conn: &mut PgConnection,
        reference: &str,
        user_id: Uuid,
        lock: bool,
    ) -> AppResult<PollAccess> {
        let not_found = || AppError::not_found("POLL_NOT_FOUND", "poll not found");
        let lock = if lock { " FOR UPDATE" } else { "" };
        let poll: Option<PollRow> = match Ref::parse(reference)? {
            Ref::Id(id) => {
                sqlx::query_as(sqlx::AssertSqlSafe(format!(
                    "SELECT {} FROM date_polls q WHERE q.id = $1{lock}",
                    poll_columns!()
                )))
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
            }
            Ref::Short(short_id) => {
                sqlx::query_as(sqlx::AssertSqlSafe(format!(
                    "SELECT {} FROM date_polls q WHERE q.short_id = $1{lock}",
                    poll_columns!()
                )))
                .bind(short_id)
                .fetch_optional(&mut *conn)
                .await?
            }
        };
        let poll = poll.ok_or_else(not_found)?;
        let joined: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM date_poll_participants WHERE poll_id = $1 AND user_id = $2)",
        )
        .bind(poll.id)
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        // The link only opens the poll for friends of its creator.
        let visible = joined
            || poll.creator_id == user_id
            || (poll.open_to_friends && graph::are_friends(&self.core, poll.creator_id, user_id).await?);
        if visible { Ok(PollAccess { poll, joined }) } else { Err(not_found()) }
    }

    /// One poll, in full, as `viewer` sees it.
    async fn poll_for(&self, row: PollRow, viewer: Uuid) -> AppResult<Poll> {
        Ok(self.polls_to_proto(vec![row], viewer, true).await?.remove(0))
    }

    /// Builds the API form of polls `viewer` may see; `full` adds the participants.
    async fn polls_to_proto(&self, rows: Vec<PollRow>, viewer: Uuid, full: bool) -> AppResult<Vec<Poll>> {
        let core = &self.core;
        let poll_ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        let slots: Vec<SlotRow> =
            sqlx::query_as("SELECT id, poll_id, start_at, slot_date, end_at, sort_at FROM date_poll_slots WHERE poll_id = ANY($1) ORDER BY poll_id, sort_at, id").bind(&poll_ids).fetch_all(&core.db).await?;
        let answers: Vec<AnswerRow> =
            sqlx::query_as("SELECT poll_id, slot_id, user_id, answer FROM date_poll_answers WHERE poll_id = ANY($1)")
                .bind(&poll_ids)
                .fetch_all(&core.db)
                .await?;
        // People who answered first, in the order they answered; then the rest.
        let participants: Vec<PollParticipantRow> = sqlx::query_as(
            "SELECT poll_id, user_id, invited, comment, answered_at FROM date_poll_participants WHERE poll_id = ANY($1)
             ORDER BY poll_id, answered_at NULLS LAST, created_at, user_id",
        )
        .bind(&poll_ids)
        .fetch_all(&core.db)
        .await?;

        let mut needed: HashSet<Uuid> = rows.iter().map(|row| row.creator_id).collect();
        if full {
            needed.extend(participants.iter().map(|participant| participant.user_id));
        }
        let people = People::load(core, viewer, needed).await?;

        Ok(rows
            .into_iter()
            .map(|row| {
                let slots: Vec<&SlotRow> = slots.iter().filter(|slot| slot.poll_id == row.id).collect();
                let answers: Vec<AnswerRow> =
                    answers.iter().filter(|answer| answer.poll_id == row.id).cloned().collect();
                let participants: Vec<&PollParticipantRow> =
                    participants.iter().filter(|participant| participant.poll_id == row.id).collect();
                let owned_slots: Vec<SlotRow> = slots.iter().map(|slot| (*slot).clone()).collect();
                let tallies = tallies_of(&owned_slots, &answers);
                let my_answer = |slot_id: Uuid| {
                    answers
                        .iter()
                        .find(|answer| answer.slot_id == slot_id && answer.user_id == viewer)
                        .map(|answer| answer.answer as i32)
                        .unwrap_or(0)
                };
                Poll {
                    id: row.id.to_string(),
                    short_id: row.short_id.clone(),
                    title: row.title.clone(),
                    description: row.description.clone(),
                    creator: people.summary(row.creator_id).into(),
                    time_zone: row.time_zone.clone(),
                    slots: slots
                        .iter()
                        .zip(&tallies)
                        .map(|(slot, tally)| PollSlot {
                            id: slot.id.to_string(),
                            start: Some(match slot.candidate() {
                                Candidate::At { start, .. } => poll_slot::Start::StartAt(Box::new(start.into())),
                                Candidate::On(date) => poll_slot::Start::Date(dates::format_date(date)),
                            }),
                            end_at: opt_ts(slot.end_at),
                            yes_count: tally.yes,
                            maybe_count: tally.maybe,
                            no_count: tally.no,
                            my_answer: my_answer(slot.id).into(),
                            ..Default::default()
                        })
                        .collect(),
                    status: row.status().into(),
                    open_to_friends: row.open_to_friends,
                    deadline: opt_ts(row.deadline),
                    participants: if full {
                        participants
                            .iter()
                            .map(|participant| {
                                let hidden = people.is_hidden(participant.user_id);
                                PollParticipant {
                                    user: people.summary(participant.user_id).into(),
                                    invited: participant.invited,
                                    answers: answers
                                        .iter()
                                        .filter(|answer| answer.user_id == participant.user_id)
                                        .map(|answer| SlotAnswer {
                                            slot_id: answer.slot_id.to_string(),
                                            answer: (answer.answer as i32).into(),
                                            ..Default::default()
                                        })
                                        .collect(),
                                    comment: if hidden { String::new() } else { participant.comment.clone() },
                                    answered_at: opt_ts(participant.answered_at),
                                    hidden,
                                    ..Default::default()
                                }
                            })
                            .collect()
                    } else {
                        Vec::new()
                    },
                    participant_count: participants.len() as i32,
                    answered_count: participants.iter().filter(|participant| participant.answered_at.is_some()).count()
                        as i32,
                    best_slot_id: best_slot(&tallies).map(|id| id.to_string()).unwrap_or_default(),
                    final_slot_id: row.final_slot_id.map(|id| id.to_string()).unwrap_or_default(),
                    plan_id: row.plan_id.map(|id| id.to_string()).unwrap_or_default(),
                    mine: row.creator_id == viewer,
                    answered_by_me: participants
                        .iter()
                        .any(|participant| participant.user_id == viewer && participant.answered_at.is_some()),
                    created_at: ts(row.created_at),
                    updated_at: ts(row.updated_at),
                    ..Default::default()
                }
            })
            .collect())
    }

    /// Pushes the state of a poll (status, tallies, suggestion) to everyone in it.
    async fn push_poll(&self, poll_id: Uuid, details_changed: bool) -> AppResult<()> {
        let mut conn = self.core.db.acquire().await?;
        let Some(poll): Option<PollRow> =
            sqlx::query_as(concat!("SELECT ", poll_columns!(), " FROM date_polls q WHERE q.id = $1"))
                .bind(poll_id)
                .fetch_optional(&mut *conn)
                .await?
        else {
            return Ok(());
        };
        let slots = slots_of(&mut conn, poll_id).await?;
        let answers: Vec<AnswerRow> =
            sqlx::query_as("SELECT poll_id, slot_id, user_id, answer FROM date_poll_answers WHERE poll_id = $1")
                .bind(poll_id)
                .fetch_all(&mut *conn)
                .await?;
        let answered: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM date_poll_participants WHERE poll_id = $1 AND answered_at IS NOT NULL",
        )
        .bind(poll_id)
        .fetch_one(&mut *conn)
        .await?;
        let recipients = poll_participant_ids(&mut conn, poll_id).await?;
        drop(conn);
        let tallies = tallies_of(&slots, &answers);
        let event = PollChanged {
            poll_id: poll_id.to_string(),
            status: poll.status().into(),
            tallies: tallies
                .iter()
                .map(|tally| PollSlotTally {
                    slot_id: tally.slot_id.to_string(),
                    yes_count: tally.yes,
                    maybe_count: tally.maybe,
                    no_count: tally.no,
                    ..Default::default()
                })
                .collect(),
            answered_count: answered as i32,
            best_slot_id: best_slot(&tallies).map(|id| id.to_string()).unwrap_or_default(),
            plan_id: poll.plan_id.map(|id| id.to_string()).unwrap_or_default(),
            details_changed,
            ..Default::default()
        };
        self.push(recipients, Push::PollChanged(Box::new(event))).await;
        Ok(())
    }

    /// Adds invited participants and tells them.
    async fn invite_to_poll_rows(&self, conn: &mut PgConnection, poll: &PollRow, invitees: &[Uuid]) -> AppResult<()> {
        if invitees.is_empty() {
            return Ok(());
        }
        // Someone who had joined through the link simply becomes "invited".
        sqlx::query(
            "INSERT INTO date_poll_participants (poll_id, user_id, invited) SELECT $1, invitee, true FROM unnest($2::uuid[]) AS invitee
             ON CONFLICT (poll_id, user_id) DO UPDATE SET invited = true",
        )
        .bind(poll.id)
        .bind(invitees)
        .execute(&mut *conn)
        .await?;
        let by = tokuly_identity::summary(&self.core, poll.creator_id).await?;
        let text = copy::poll_invited(&name_of(&by), &poll.title);
        for invitee in invitees {
            let request = NotificationRequest::new(
                *invitee,
                "plans.poll_invited",
                text.title.clone(),
                text.body.clone(),
                poll_path(&poll.short_id),
            )
            .actor(poll.creator_id)
            .collapse(format!("plans.poll_invited:{}", poll.id))
            .data(serde_json::json!({ "poll_id": poll.id }));
            outbound::notify(&mut *conn, &request).await?;
        }
        Ok(())
    }

    /// Starts a date poll.
    pub(crate) async fn create_poll(&self, user_id: Uuid, req: &CreatePollRequest) -> AppResult<Poll> {
        let core = &self.core;
        core.cache.rate_limit(&format!("plans:poll:create:{user_id}"), 20, StdDuration::from_secs(3600)).await?;
        let now = now();
        let zone = input::zone(&req.time_zone, dates::zone_of(core, user_id).await?)?;
        let title = input::required_line(&req.title, MAX_TITLE, "TITLE_REQUIRED", "TITLE_TOO_LONG")?;
        let description = input::paragraph(&req.description, MAX_DESCRIPTION, "DESCRIPTION_TOO_LONG")?;
        ensure!(!req.slots.is_empty(), invalid, "SLOTS_REQUIRED", "a poll needs at least one candidate");
        ensure!(req.slots.len() <= MAX_SLOTS, invalid, "TOO_MANY_SLOTS", "a poll holds at most {MAX_SLOTS} candidates");
        let candidates = parse_slots(&req.slots, zone, now)?;
        let deadline = parse_deadline(req.deadline.as_option().map(from_ts), now)?;
        let invitees = input::people(&req.invitee_ids, user_id)?;
        ensure!(
            (invitees.len() as i64) < self.cfg.max_participants,
            invalid,
            "TOO_MANY_PARTICIPANTS",
            "a poll holds at most {} people",
            self.cfg.max_participants
        );
        people::require_friends(core, user_id, &invitees).await?;

        let mut tx = core.db.begin().await?;
        let row: PollRow = sqlx::query_as(concat!(
            "INSERT INTO date_polls AS q (short_id, creator_id, title, description, time_zone, open_to_friends, deadline, remind_due_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING ",
            poll_columns!()
        ))
        .bind(new_short_id())
        .bind(user_id)
        .bind(&title)
        .bind(&description)
        .bind(zone.name())
        .bind(req.open_to_friends)
        .bind(deadline)
        .bind(remind_due(deadline, now))
        .fetch_one(&mut *tx)
        .await?;
        insert_slots(&mut tx, row.id, &candidates, zone).await?;
        sqlx::query("INSERT INTO date_poll_participants (poll_id, user_id) VALUES ($1, $2)")
            .bind(row.id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        self.invite_to_poll_rows(&mut tx, &row, &invitees).await?;
        let created = PollCreated {
            poll_id: row.id,
            creator_id: user_id,
            invitee_ids: invitees.clone(),
            slot_count: candidates.len() as i32,
        };
        bus::emit(&mut *tx, POLL_CREATED, Some(user_id), &created).await?;
        tx.commit().await?;

        self.push_poll(row.id, true).await?;
        self.poll_for(row, user_id).await
    }

    /// A poll by ID or by its link.
    pub(crate) async fn get_poll(&self, user_id: Uuid, reference: &str) -> AppResult<Poll> {
        let mut conn = self.core.db.acquire().await?;
        let access = self.poll_access(&mut conn, reference, user_id, false).await?;
        drop(conn);
        self.poll_for(access.poll, user_id).await
    }

    /// Changes the texts, the deadline, the link setting or the slots of an open poll.
    pub(crate) async fn update_poll(&self, user_id: Uuid, req: &UpdatePollRequest) -> AppResult<Poll> {
        let now = now();
        let mut tx = self.core.db.begin().await?;
        let poll = self.poll_access(&mut tx, &req.poll_id, user_id, true).await?.poll;
        poll.ensure_creator(user_id)?;
        poll.ensure_open()?;
        let zone = dates::zone(&poll.time_zone);

        let title = req
            .title
            .as_deref()
            .map(|title| input::required_line(title, MAX_TITLE, "TITLE_REQUIRED", "TITLE_TOO_LONG"))
            .transpose()?
            .unwrap_or_else(|| poll.title.clone());
        let description = req
            .description
            .as_deref()
            .map(|text| input::paragraph(text, MAX_DESCRIPTION, "DESCRIPTION_TOO_LONG"))
            .transpose()?
            .unwrap_or_else(|| poll.description.clone());
        let open_to_friends = req.open_to_friends.unwrap_or(poll.open_to_friends);
        let deadline = if req.clear_deadline {
            None
        } else if req.deadline.is_set() {
            parse_deadline(req.deadline.as_option().map(from_ts), now)?
        } else {
            poll.deadline
        };

        let existing = slots_of(&mut tx, poll.id).await?;
        let removed = parse_ids(&req.remove_slot_ids)?;
        ensure!(
            removed.iter().all(|id| existing.iter().any(|slot| slot.id == *id)),
            not_found,
            "SLOT_NOT_FOUND",
            "a slot to remove is not part of the poll"
        );
        let added = parse_slots(&req.add_slots, zone, now)?;
        let kept: Vec<Candidate> =
            existing.iter().filter(|slot| !removed.contains(&slot.id)).map(SlotRow::candidate).collect();
        ensure!(
            added.iter().all(|candidate| !kept.contains(candidate)),
            invalid,
            "SLOT_DUPLICATE",
            "the poll already has this slot"
        );
        ensure!(kept.len() + added.len() >= 1, invalid, "SLOTS_REQUIRED", "a poll needs at least one candidate");
        ensure!(
            kept.len() + added.len() <= MAX_SLOTS,
            invalid,
            "TOO_MANY_SLOTS",
            "a poll holds at most {MAX_SLOTS} candidates"
        );

        let row: PollRow = sqlx::query_as(concat!(
            "UPDATE date_polls q SET title = $2, description = $3, open_to_friends = $4, deadline = $5,
                    remind_due_at = CASE WHEN $6 THEN $7 ELSE remind_due_at END
             WHERE q.id = $1 RETURNING ",
            poll_columns!()
        ))
        .bind(poll.id)
        .bind(&title)
        .bind(&description)
        .bind(open_to_friends)
        .bind(deadline)
        .bind(deadline != poll.deadline)
        .bind(remind_due(deadline, now))
        .fetch_one(&mut *tx)
        .await?;
        // Answers to a removed slot go with it.
        sqlx::query("DELETE FROM date_poll_slots WHERE poll_id = $1 AND id = ANY($2)")
            .bind(poll.id)
            .bind(&removed)
            .execute(&mut *tx)
            .await?;
        insert_slots(&mut tx, poll.id, &added, zone).await?;
        tx.commit().await?;

        self.push_poll(poll.id, true).await?;
        self.poll_for(row, user_id).await
    }

    /// Polls `user_id` created or takes part in: open ones first, newest first.
    pub(crate) async fn list_polls(&self, user_id: Uuid, page: &Page) -> AppResult<(Vec<Poll>, PageInfo)> {
        let after: Option<(i32, Uuid)> = page.decode()?;
        let mut rows: Vec<PollRow> = sqlx::query_as(concat!(
            "SELECT ",
            poll_columns!(),
            " FROM date_polls q JOIN date_poll_participants me ON me.poll_id = q.id AND me.user_id = $1
              WHERE $2::int IS NULL OR (CASE WHEN q.status = 1 THEN 0 ELSE 1 END) > $2
                 OR ((CASE WHEN q.status = 1 THEN 0 ELSE 1 END) = $2 AND q.id < $3)
              ORDER BY (CASE WHEN q.status = 1 THEN 0 ELSE 1 END), q.id DESC
              LIMIT $4"
        ))
        .bind(user_id)
        .bind(after.map(|(rank, _)| rank))
        .bind(after.map(|(_, id)| id))
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (i32::from(!row.is_open()), row.id));
        Ok((self.polls_to_proto(rows, user_id, false).await?, info))
    }

    /// Invites more friends of the creator. Returns the poll and how many were newly invited.
    pub(crate) async fn invite_to_poll(
        &self,
        user_id: Uuid,
        reference: &str,
        user_ids: &[String],
    ) -> AppResult<(Poll, i32)> {
        let core = &self.core;
        core.cache.rate_limit(&format!("plans:invite:{user_id}"), 300, StdDuration::from_secs(3600)).await?;
        let wanted = input::people(user_ids, user_id)?;

        let mut tx = core.db.begin().await?;
        let poll = self.poll_access(&mut tx, reference, user_id, true).await?.poll;
        poll.ensure_creator(user_id)?;
        poll.ensure_open()?;
        people::require_friends(core, user_id, &wanted).await?;
        let invited: HashSet<Uuid> =
            sqlx::query_scalar("SELECT user_id FROM date_poll_participants WHERE poll_id = $1 AND invited")
                .bind(poll.id)
                .fetch_all(&mut *tx)
                .await?
                .into_iter()
                .collect();
        let invitees: Vec<Uuid> = wanted.into_iter().filter(|id| !invited.contains(id)).collect();
        let total: i64 = sqlx::query_scalar("SELECT count(*) FROM date_poll_participants WHERE poll_id = $1")
            .bind(poll.id)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(
            total + invitees.len() as i64 <= self.cfg.max_participants,
            invalid,
            "TOO_MANY_PARTICIPANTS",
            "a poll holds at most {} people",
            self.cfg.max_participants
        );
        self.invite_to_poll_rows(&mut tx, &poll, &invitees).await?;
        tx.commit().await?;

        if !invitees.is_empty() {
            self.push_poll(poll.id, true).await?;
        }
        Ok((self.poll_for(poll, user_id).await?, invitees.len() as i32))
    }

    /// Answers ○ △ × per slot, replacing the caller's previous answers.
    pub(crate) async fn answer_poll(&self, user_id: Uuid, req: &AnswerPollRequest) -> AppResult<Poll> {
        let comment = input::line(&req.comment, MAX_COMMENT, "COMMENT_TOO_LONG")?;
        // Every answer redraws the tallies on everyone's screen.
        self.core.cache.rate_limit(&format!("plans:poll:answer:{user_id}"), 60, StdDuration::from_secs(60)).await?;
        let mut tx = self.core.db.begin().await?;
        let access = self.poll_access(&mut tx, &req.poll_id, user_id, true).await?;
        let poll = access.poll;
        poll.ensure_open()?;

        let slots = slots_of(&mut tx, poll.id).await?;
        let mut seen = HashSet::new();
        let mut given: Vec<(Uuid, i16)> = Vec::with_capacity(req.answers.len());
        for answer in &req.answers {
            let slot_id = parse_id(&answer.slot_id)?;
            ensure!(
                slots.iter().any(|slot| slot.id == slot_id),
                not_found,
                "SLOT_NOT_FOUND",
                "this slot is not part of the poll"
            );
            ensure!(seen.insert(slot_id), invalid, "SLOT_DUPLICATE", "one answer per slot");
            let value = match answer.answer.as_known() {
                Some(value) if value != PollAnswer::POLL_ANSWER_UNSPECIFIED => value,
                _ => bail!(invalid, "ANSWER_INVALID", "answer yes, maybe or no"),
            };
            given.push((slot_id, value as i32 as i16));
        }

        // Answering through the link is what joins the poll.
        if !access.joined {
            let total: i64 = sqlx::query_scalar("SELECT count(*) FROM date_poll_participants WHERE poll_id = $1")
                .bind(poll.id)
                .fetch_one(&mut *tx)
                .await?;
            ensure!(
                total < self.cfg.max_participants,
                precondition,
                "TOO_MANY_PARTICIPANTS",
                "a poll holds at most {} people",
                self.cfg.max_participants
            );
            sqlx::query("INSERT INTO date_poll_participants (poll_id, user_id) VALUES ($1, $2)")
                .bind(poll.id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
        }
        let answered_before: bool = sqlx::query_scalar(
            "SELECT answered_at IS NOT NULL FROM date_poll_participants WHERE poll_id = $1 AND user_id = $2",
        )
        .bind(poll.id)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query("DELETE FROM date_poll_answers WHERE poll_id = $1 AND user_id = $2")
            .bind(poll.id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        let (slot_ids, values): (Vec<Uuid>, Vec<i16>) = given.iter().copied().unzip();
        sqlx::query("INSERT INTO date_poll_answers (slot_id, user_id, poll_id, answer) SELECT slot, $2, $1, answer FROM unnest($3::uuid[], $4::int2[]) AS given(slot, answer)")
            .bind(poll.id)
            .bind(user_id)
            .bind(&slot_ids)
            .bind(&values)
            .execute(&mut *tx)
            .await?;
        // Taking every answer back (and saying nothing) is "not answered" again.
        let answered = !given.is_empty() || !comment.is_empty();
        sqlx::query("UPDATE date_poll_participants SET comment = $3, answered_at = CASE WHEN $4 THEN now() END WHERE poll_id = $1 AND user_id = $2")
            .bind(poll.id)
            .bind(user_id)
            .bind(&comment)
            .bind(answered)
            .execute(&mut *tx)
            .await?;
        if answered && !answered_before && poll.creator_id != user_id {
            let by = tokuly_identity::summary(&self.core, user_id).await?;
            let text = copy::poll_answered(&name_of(&by), &poll.title);
            let request = NotificationRequest::new(
                poll.creator_id,
                "plans.poll_answer",
                text.title,
                text.body,
                poll_path(&poll.short_id),
            )
            .actor(user_id)
            .collapse(format!("plans.poll_answer:{}", poll.id))
            .data(serde_json::json!({ "poll_id": poll.id }));
            outbound::notify(&mut *tx, &request).await?;
        }
        tx.commit().await?;

        self.push_poll(poll.id, !access.joined).await?;
        self.poll_for(poll, user_id).await
    }

    /// Stops taking answers, or takes them again.
    pub(crate) async fn set_poll_closed(&self, user_id: Uuid, reference: &str, closed: bool) -> AppResult<Poll> {
        let mut tx = self.core.db.begin().await?;
        let poll = self.poll_access(&mut tx, reference, user_id, true).await?.poll;
        poll.ensure_creator(user_id)?;
        let row: PollRow = if closed {
            poll.ensure_open()?;
            sqlx::query_as(concat!(
                "UPDATE date_polls q SET status = 2, remind_due_at = NULL WHERE q.id = $1 RETURNING ",
                poll_columns!()
            ))
            .bind(poll.id)
            .fetch_one(&mut *tx)
            .await?
        } else {
            ensure!(
                poll.status() == PollStatus::POLL_STATUS_CLOSED,
                precondition,
                "POLL_NOT_CLOSED",
                "only a closed poll that was not finalized can be reopened"
            );
            // A deadline that has passed would close it again right away.
            sqlx::query_as(concat!("UPDATE date_polls q SET status = 1, deadline = CASE WHEN deadline > now() THEN deadline END WHERE q.id = $1 RETURNING ", poll_columns!()))
                .bind(poll.id)
                .fetch_one(&mut *tx)
                .await?
        };
        tx.commit().await?;
        self.push_poll(poll.id, false).await?;
        self.poll_for(row, user_id).await
    }

    /// Picks a slot and turns the poll into a plan hosted by its creator, with
    /// everyone who answered ○ or △ for that slot invited.
    pub(crate) async fn finalize_poll(&self, user_id: Uuid, req: &FinalizePollRequest) -> AppResult<(Poll, Plan)> {
        let core = &self.core;
        let emoji = input::emoji(&req.emoji)?;
        let place = Place::parse(req.place.as_option())?;

        let mut tx = core.db.begin().await?;
        let poll = self.poll_access(&mut tx, &req.poll_id, user_id, true).await?.poll;
        poll.ensure_creator(user_id)?;
        ensure!(
            poll.status() != PollStatus::POLL_STATUS_FINALIZED,
            precondition,
            "POLL_FINALIZED",
            "the poll was already turned into a plan"
        );
        let zone = dates::zone(&poll.time_zone);
        let description = if req.description.trim().is_empty() {
            poll.description.clone()
        } else {
            plan_description(&req.description)?
        };

        let slots = slots_of(&mut tx, poll.id).await?;
        let answers: Vec<AnswerRow> =
            sqlx::query_as("SELECT poll_id, slot_id, user_id, answer FROM date_poll_answers WHERE poll_id = $1")
                .bind(poll.id)
                .fetch_all(&mut *tx)
                .await?;
        let slot_id = match parse_opt_id(&req.slot_id)? {
            Some(slot_id) => slot_id,
            None => best_slot(&tallies_of(&slots, &answers))
                .ok_or_else(|| AppError::invalid("SLOT_REQUIRED", "nobody has answered yet: choose a slot"))?,
        };
        let slot = slots
            .iter()
            .find(|slot| slot.id == slot_id)
            .or_not_found("SLOT_NOT_FOUND", "this slot is not part of the poll")?;

        // "The people who could make it": ○ or △ for the slot, as far as they
        // are still friends of the host.
        let friends: HashSet<Uuid> = graph::friend_ids(core, user_id).await?.into_iter().collect();
        let could_make_it =
            |answer: &AnswerRow| answer.slot_id == slot_id && answer.answer != PollAnswer::POLL_ANSWER_NO as i32 as i16;
        let mut invitees: Vec<Uuid> = answers
            .iter()
            .filter(|answer| could_make_it(answer) && answer.user_id != user_id && friends.contains(&answer.user_id))
            .map(|answer| answer.user_id)
            .collect();
        invitees.sort();

        let when = match slot.candidate() {
            Candidate::At { start, end } => When::At { start, end },
            Candidate::On(date) => When::On { start: date, end: None },
        };
        let new = NewPlan {
            title: poll.title.clone(),
            emoji,
            description,
            schedule: Schedule { when, zone },
            place,
            guests_can_invite: false,
            capacity: 0,
            remind_day_before: true,
            remind_minutes_before: 60,
            currency: "JPY".to_owned(),
            poll_id: Some(poll.id),
        };
        let plan = self.insert_plan(&mut tx, user_id, &new, &invitees).await?;
        let row: PollRow = sqlx::query_as(concat!("UPDATE date_polls q SET status = 3, final_slot_id = $2, plan_id = $3, remind_due_at = NULL WHERE q.id = $1 RETURNING ", poll_columns!()))
            .bind(poll.id)
            .bind(slot_id)
            .bind(plan.id)
            .fetch_one(&mut *tx)
            .await?;
        let answered: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM date_poll_participants WHERE poll_id = $1 AND answered_at IS NOT NULL",
        )
        .bind(poll.id)
        .fetch_one(&mut *tx)
        .await?;
        let finalized =
            PollFinalized { poll_id: poll.id, creator_id: user_id, plan_id: plan.id, answered_count: answered as i32 };
        bus::emit(&mut *tx, POLL_FINALIZED, Some(user_id), &finalized).await?;
        tx.commit().await?;

        self.push_poll(poll.id, false).await?;
        self.push(invitees, plan_changed(plan.id, PlanChange::PLAN_CHANGE_INVITED)).await;
        Ok((self.poll_for(row, user_id).await?, self.plan_for(plan, user_id).await?))
    }

    /// Deletes a poll. A plan made from it stays.
    pub(crate) async fn delete_poll(&self, user_id: Uuid, reference: &str) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let poll = self.poll_access(&mut tx, reference, user_id, true).await?.poll;
        poll.ensure_creator(user_id)?;
        let everyone = poll_participant_ids(&mut tx, poll.id).await?;
        sqlx::query("DELETE FROM date_polls WHERE id = $1").bind(poll.id).execute(&mut *tx).await?;
        tx.commit().await?;
        self.push(everyone, poll_deleted(poll.id)).await;
        Ok(())
    }

    /// Reminds the invited people who have not answered. Returns how many.
    async fn remind_unanswered(&self, conn: &mut PgConnection, poll: &PollRow) -> AppResult<i32> {
        let waiting: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM date_poll_participants WHERE poll_id = $1 AND invited AND answered_at IS NULL AND user_id <> $2")
            .bind(poll.id)
            .bind(poll.creator_id)
            .fetch_all(&mut *conn)
            .await?;
        let text = copy::poll_reminder(&poll.title);
        for user_id in &waiting {
            let request = NotificationRequest::new(
                *user_id,
                "plans.poll_reminder",
                text.title.clone(),
                text.body.clone(),
                poll_path(&poll.short_id),
            )
            .actor(poll.creator_id)
            .collapse(format!("plans.poll_reminder:{}", poll.id))
            .data(serde_json::json!({ "poll_id": poll.id }));
            outbound::notify(&mut *conn, &request).await?;
        }
        Ok(waiting.len() as i32)
    }

    /// The creator nudges the people who have not answered.
    pub(crate) async fn remind_poll_participants(&self, user_id: Uuid, reference: &str) -> AppResult<i32> {
        let mut tx = self.core.db.begin().await?;
        let poll = self.poll_access(&mut tx, reference, user_id, true).await?.poll;
        poll.ensure_creator(user_id)?;
        poll.ensure_open()?;
        self.core
            .cache
            .rate_limit(&format!("plans:poll:remind:{}", poll.id), 1, StdDuration::from_secs(12 * 3600))
            .await?;
        let reminded = self.remind_unanswered(&mut tx, &poll).await?;
        tx.commit().await?;
        Ok(reminded)
    }

    /// Closes the polls whose deadline has passed and tells their creators.
    /// Returns how many were closed.
    pub(crate) async fn close_due_polls(&self) -> AppResult<u64> {
        let mut tx = self.core.db.begin().await?;
        let closed: Vec<PollRow> = sqlx::query_as(concat!(
            "UPDATE date_polls q SET status = 2, remind_due_at = NULL
             WHERE q.id IN (SELECT id FROM date_polls WHERE status = 1 AND deadline <= now() ORDER BY deadline LIMIT 100 FOR UPDATE SKIP LOCKED)
             RETURNING ",
            poll_columns!()
        ))
        .fetch_all(&mut *tx)
        .await?;
        for poll in &closed {
            let text = copy::poll_closed(&poll.title);
            let request = NotificationRequest::new(
                poll.creator_id,
                "plans.poll_closed",
                text.title,
                text.body,
                poll_path(&poll.short_id),
            )
            .collapse(format!("plans.poll_closed:{}", poll.id))
            .data(serde_json::json!({ "poll_id": poll.id }));
            outbound::notify(&mut *tx, &request).await?;
        }
        tx.commit().await?;
        for poll in &closed {
            self.push_poll(poll.id, false).await?;
        }
        Ok(closed.len() as u64)
    }

    /// Sends the automatic reminder of the polls for which it is due.
    /// Returns how many polls were handled.
    pub(crate) async fn remind_due_polls(&self) -> AppResult<u64> {
        let mut tx = self.core.db.begin().await?;
        let due: Vec<PollRow> = sqlx::query_as(concat!(
            "UPDATE date_polls q SET remind_due_at = NULL
             WHERE q.id IN (SELECT id FROM date_polls WHERE remind_due_at <= now() ORDER BY remind_due_at LIMIT 100 FOR UPDATE SKIP LOCKED)
             RETURNING ",
            poll_columns!()
        ))
        .fetch_all(&mut *tx)
        .await?;
        for poll in due.iter().filter(|poll| poll.is_open()) {
            self.remind_unanswered(&mut tx, poll).await?;
        }
        tx.commit().await?;
        Ok(due.len() as u64)
    }
}

/// The realtime event saying that a poll is gone.
pub(crate) fn poll_deleted(poll_id: Uuid) -> Push {
    Push::PollChanged(Box::new(PollChanged { poll_id: poll_id.to_string(), deleted: true, ..Default::default() }))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    const TOKYO: Tz = chrono_tz::Asia::Tokyo;

    fn tokyo(month: u32, day: u32, hour: u32) -> Time {
        TOKYO.with_ymd_and_hms(2026, month, day, hour, 0, 0).unwrap().with_timezone(&chrono::Utc)
    }

    fn tally(n: u128, day: u32, yes: i32, maybe: i32, no: i32) -> Tally {
        Tally { slot_id: Uuid::from_u128(n), sort_at: tokyo(12, day, 0), yes, maybe, no }
    }

    fn timed(start: Time, end: Option<Time>) -> PollSlotInput {
        PollSlotInput {
            start: Some(poll_slot_input::Start::StartAt(Box::new(start.into()))),
            end_at: opt_ts(end),
            ..Default::default()
        }
    }

    fn day(date: &str) -> PollSlotInput {
        PollSlotInput { start: Some(poll_slot_input::Start::Date(date.to_owned())), ..Default::default() }
    }

    #[test]
    fn the_suggestion_is_most_yes_then_fewest_no_then_earliest() {
        // Most ○ wins outright.
        assert_eq!(best_slot(&[tally(1, 5, 2, 0, 0), tally(2, 6, 3, 0, 2)]), Some(Uuid::from_u128(2)));
        // Same ○: fewer × wins, whatever the △.
        assert_eq!(best_slot(&[tally(1, 5, 3, 0, 2), tally(2, 6, 3, 1, 1)]), Some(Uuid::from_u128(2)));
        // Same ○ and ×: the earlier slot wins.
        assert_eq!(best_slot(&[tally(1, 7, 3, 0, 1), tally(2, 6, 3, 2, 1)]), Some(Uuid::from_u128(2)));
        // Nobody answered: nothing to suggest.
        assert_eq!(best_slot(&[tally(1, 5, 0, 0, 0), tally(2, 6, 0, 0, 0)]), None);
        assert_eq!(best_slot(&[]), None);
        // Only × so far still makes the least-refused slot the suggestion.
        assert_eq!(best_slot(&[tally(1, 5, 0, 0, 2), tally(2, 6, 0, 0, 1)]), Some(Uuid::from_u128(2)));
    }

    #[test]
    fn slots_are_days_or_times_in_the_near_future() {
        let now = tokyo(11, 22, 12);
        let slots = parse_slots(
            &[day("2026-12-05"), timed(tokyo(12, 6, 18), Some(tokyo(12, 6, 21))), timed(tokyo(12, 7, 18), None)],
            TOKYO,
            now,
        )
        .unwrap();
        assert_eq!(
            slots,
            vec![
                Candidate::On(NaiveDate::from_ymd_opt(2026, 12, 5).unwrap()),
                Candidate::At { start: tokyo(12, 6, 18), end: Some(tokyo(12, 6, 21)) },
                Candidate::At { start: tokyo(12, 7, 18), end: None },
            ]
        );
        // A day sorts at its local midnight.
        assert_eq!(slots[0].sort_at(TOKYO), tokyo(12, 5, 0));

        let reason = |inputs: &[PollSlotInput]| parse_slots(inputs, TOKYO, now).unwrap_err().reason.to_string();
        assert_eq!(reason(&[PollSlotInput::default()]), "SLOT_INVALID");
        assert_eq!(reason(&[day("12/05")]), "SLOT_INVALID");
        assert_eq!(reason(&[day("2026-11-01")]), "SLOT_INVALID");
        assert_eq!(reason(&[day("2029-12-05")]), "SLOT_INVALID");
        assert_eq!(reason(&[timed(tokyo(12, 6, 18), Some(tokyo(12, 6, 18)))]), "SLOT_INVALID");
        let day_with_end = PollSlotInput { end_at: ts(tokyo(12, 6, 21)), ..day("2026-12-05") };
        assert_eq!(reason(&[day_with_end]), "SLOT_INVALID");
        assert_eq!(reason(&[day("2026-12-05"), day("2026-12-05")]), "SLOT_DUPLICATE");
    }

    #[test]
    fn deadlines_lie_ahead() {
        let now = tokyo(11, 22, 12);
        assert_eq!(parse_deadline(None, now).unwrap(), None);
        assert_eq!(parse_deadline(Some(tokyo(11, 25, 12)), now).unwrap(), Some(tokyo(11, 25, 12)));
        for bad in [now, now - Duration::hours(1), now + Duration::days(400)] {
            assert_eq!(parse_deadline(Some(bad), now).unwrap_err().reason, "DEADLINE_INVALID");
        }
    }

    #[test]
    fn the_automatic_reminder_comes_a_day_before_the_deadline() {
        let now = tokyo(11, 22, 12);
        assert_eq!(remind_due(Some(tokyo(11, 25, 12)), now), Some(tokyo(11, 24, 12)));
        // Too close to the deadline for a reminder to make sense.
        assert_eq!(remind_due(Some(tokyo(11, 23, 12)), now), None);
        assert_eq!(remind_due(Some(tokyo(11, 23, 13)), now), Some(tokyo(11, 22, 13)));
        // No deadline: two days after the poll was made.
        assert_eq!(remind_due(None, now), Some(tokyo(11, 24, 12)));
    }
}
