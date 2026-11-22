//! "Days since" counters shared by two people: a friendship, or a couple.
//!
//! A friendship counter exists as soon as one of two friends starts it, and
//! both see it. A couple counter starts as an invitation and only exists
//! once the other person accepts; a user is in at most one couple. Either of
//! the two can change or end a counter. Nobody else sees it — except that a
//! couple counter appears on both profiles when both chose to show it there.
//!
//! Milestones (see [`crate::milestones`]) are announced to each of the two
//! on the morning of the day in their own time zone.
use std::time::Duration as StdDuration;

use buffa::Enumeration as _;
use chrono::NaiveDate;
use chrono_tz::Tz;
use sqlx::PgConnection;
use tokuly_core::{
    events as bus,
    outbound::{self, NotificationRequest},
    prelude::*,
};
use tokuly_proto::{
    common::{LocalizedText, SubjectType, UserSummary},
    plans::{plans_event::Event as Push, *},
};
use tokuly_social::graph;

use crate::{
    Plans, copy, dates,
    events::{ANNIVERSARY_STARTED, AnniversaryStarted},
    input, milestones, people,
};

const MAX_TITLE: usize = 40;
/// Counters started with, and couple invitations sent to, the same friend in
/// a day: each of them notifies that friend.
const PER_FRIEND_PER_DAY: u32 = 3;
/// Friendship counters one person may start with the same friend in a day.
const COUNTERS_PER_FRIEND_PER_DAY: u32 = 5;
/// Milestones listed ahead on a counter.
const UPCOMING_MILESTONES: usize = 3;

pub(crate) fn anniversary_path(id: Uuid) -> String {
    format!("/plans/anniversaries/{id}")
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct AnniversaryRow {
    pub id: Uuid,
    pub kind: i16,
    pub user_lo: Uuid,
    pub user_hi: Uuid,
    pub created_by: Uuid,
    pub start_date: NaiveDate,
    pub count_start_day: bool,
    pub title: String,
    pub emoji: String,
    pub status: i16,
    pub show_lo: bool,
    pub show_hi: bool,
    pub created_at: Time,
}

macro_rules! anniversary_columns {
    () => {
        "a.id, a.kind, a.user_lo, a.user_hi, a.created_by, a.start_date, a.count_start_day, a.title, a.emoji, a.status, a.show_lo, a.show_hi, a.created_at"
    };
}

impl AnniversaryRow {
    pub(crate) fn kind(&self) -> AnniversaryKind {
        AnniversaryKind::from_i32(self.kind as i32).unwrap_or(AnniversaryKind::ANNIVERSARY_KIND_UNSPECIFIED)
    }

    pub(crate) fn is_couple(&self) -> bool {
        self.kind() == AnniversaryKind::ANNIVERSARY_KIND_COUPLE
    }

    pub(crate) fn is_active(&self) -> bool {
        self.status == AnniversaryStatus::ANNIVERSARY_STATUS_ACTIVE as i32 as i16
    }

    fn has(&self, user_id: Uuid) -> bool {
        self.user_lo == user_id || self.user_hi == user_id
    }

    /// The other one of the two.
    pub(crate) fn partner_of(&self, user_id: Uuid) -> Uuid {
        if self.user_lo == user_id { self.user_hi } else { self.user_lo }
    }

    /// Whether `user_id` chose to show the counter on their profile.
    fn shown_by(&self, user_id: Uuid) -> bool {
        if self.user_lo == user_id { self.show_lo } else { self.show_hi }
    }

    /// A couple counter both chose to show on their profiles.
    pub(crate) fn on_profiles(&self) -> bool {
        self.is_couple() && self.is_active() && self.show_lo && self.show_hi
    }

    /// The count on `today`; zero before the start and for invitations.
    pub(crate) fn days(&self, today: NaiveDate) -> i64 {
        if self.is_active() && today >= self.start_date {
            milestones::day_count(self.start_date, today, self.count_start_day)
        } else {
            0
        }
    }

    /// The counter as `viewer`, one of the two, sees it on `today`.
    fn to_proto(&self, viewer: Uuid, partner: UserSummary, today: NaiveDate) -> Anniversary {
        let upcoming = if self.is_active() {
            milestones::upcoming(self.start_date, self.count_start_day, today, UPCOMING_MILESTONES)
        } else {
            Vec::new()
        };
        Anniversary {
            id: self.id.to_string(),
            kind: self.kind().into(),
            status: AnniversaryStatus::from_i32(self.status as i32)
                .unwrap_or(AnniversaryStatus::ANNIVERSARY_STATUS_UNSPECIFIED)
                .into(),
            partner: partner.into(),
            start_date: dates::format_date(self.start_date),
            count_start_day: self.count_start_day,
            days: self.days(today).clamp(0, i32::MAX as i64) as i32,
            title: self.title.clone(),
            emoji: self.emoji.clone(),
            invited_by_me: !self.is_active() && self.created_by == viewer,
            show_on_profile: self.shown_by(viewer),
            partner_shows_on_profile: self.shown_by(self.partner_of(viewer)),
            upcoming: upcoming
                .into_iter()
                .map(|milestone| {
                    let label = milestone.mark.label();
                    Milestone {
                        kind: milestone.mark.kind().into(),
                        number: milestone.mark.number(),
                        date: dates::format_date(milestone.date),
                        days_left: dates::days_until(milestone.date, today),
                        label: LocalizedText { ja: label.ja, en: label.en, ..Default::default() }.into(),
                        ..Default::default()
                    }
                })
                .collect(),
            created_at: ts(self.created_at),
            ..Default::default()
        }
    }
}

/// When the next milestone notice of a counter is due: the earliest morning,
/// in either person's time zone, of a milestone day that still lies ahead of
/// `now`.
pub(crate) fn notice_due(
    start: NaiveDate,
    count_start_day: bool,
    zones: [Tz; 2],
    now: Time,
    morning_hour: u32,
) -> Option<Time> {
    zones
        .into_iter()
        .filter_map(|zone| {
            // Today's milestone may still be ahead (before the morning hour) or
            // already past; the one after it is certainly ahead.
            milestones::upcoming(start, count_start_day, dates::day_in(zone, now), 2)
                .into_iter()
                .map(|milestone| dates::local_instant(milestone.date, morning_hour, zone))
                .find(|due| *due > now)
        })
        .min()
}

fn not_found() -> AppError {
    AppError::not_found("ANNIVERSARY_NOT_FOUND", "counter not found")
}

/// A start date written by a client: a real day, not in the future.
fn start_date(input: &str, today: NaiveDate) -> AppResult<NaiveDate> {
    let date = input::date(input, "DATE_INVALID")?;
    ensure!(date <= today, invalid, "DATE_INVALID", "the start cannot lie in the future");
    Ok(date)
}

/// Serializes everything that decides whether two people may share a
/// counter (and whether either is in a couple) for the rest of the transaction.
async fn lock_pair(conn: &mut PgConnection, a: Uuid, b: Uuid) -> AppResult<()> {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    for user_id in [lo, hi] {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("plans:anniversary:{}", user_id.simple()))
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

async fn in_couple(conn: &mut PgConnection, user_id: Uuid) -> AppResult<bool> {
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM plan_anniversaries WHERE kind = 2 AND status = 2 AND (user_lo = $1 OR user_hi = $1))").bind(user_id).fetch_one(conn).await?)
}

async fn exists_between(conn: &mut PgConnection, kind: AnniversaryKind, a: Uuid, b: Uuid) -> AppResult<bool> {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM plan_anniversaries WHERE kind = $1 AND user_lo = $2 AND user_hi = $3)",
    )
    .bind(kind as i32 as i16)
    .bind(lo)
    .bind(hi)
    .fetch_one(conn)
    .await?)
}

impl Plans {
    /// A counter `user_id` is one of the two people of.
    async fn anniversary_of(
        &self,
        conn: &mut PgConnection,
        anniversary_id: Uuid,
        user_id: Uuid,
        lock: bool,
    ) -> AppResult<AnniversaryRow> {
        let sql = if lock {
            concat!("SELECT ", anniversary_columns!(), " FROM plan_anniversaries a WHERE a.id = $1 FOR UPDATE")
        } else {
            concat!("SELECT ", anniversary_columns!(), " FROM plan_anniversaries a WHERE a.id = $1")
        };
        let row: Option<AnniversaryRow> = sqlx::query_as(sql).bind(anniversary_id).fetch_optional(conn).await?;
        row.filter(|row| row.has(user_id)).ok_or_else(not_found)
    }

    /// The counter as `viewer` sees it today.
    async fn anniversary_for(&self, row: &AnniversaryRow, viewer: Uuid) -> AppResult<Anniversary> {
        let core = &self.core;
        let partner = tokuly_identity::summary(core, row.partner_of(viewer)).await?;
        let today = dates::day_in(dates::zone_of(core, viewer).await?, now());
        Ok(row.to_proto(viewer, partner, today))
    }

    /// Pushes a counter to one of its two people, as they see it.
    async fn push_anniversary(&self, row: &AnniversaryRow, to: Uuid) -> AppResult<()> {
        let anniversary = self.anniversary_for(row, to).await?;
        self.push(
            [to],
            Push::AnniversaryChanged(Box::new(AnniversaryChanged {
                anniversary: anniversary.into(),
                ..Default::default()
            })),
        )
        .await;
        Ok(())
    }

    /// When the next milestone notice of `row` is due. The two people's time
    /// zones are read on the caller's connection (it is inside a transaction).
    async fn notice_due_on(&self, conn: &mut PgConnection, row: &AnniversaryRow) -> AppResult<Option<Time>> {
        let locals = people::locals(conn, &[row.user_lo, row.user_hi]).await?;
        let zone_of = |user_id: Uuid| locals.get(&user_id).map(|local| local.zone).unwrap_or(dates::FALLBACK_ZONE);
        Ok(notice_due(
            row.start_date,
            row.count_start_day,
            [zone_of(row.user_lo), zone_of(row.user_hi)],
            now(),
            self.cfg.morning_hour,
        ))
    }

    /// Stores a new counter between two people.
    async fn insert_anniversary(
        &self,
        conn: &mut PgConnection,
        kind: AnniversaryKind,
        status: AnniversaryStatus,
        creator: Uuid,
        other: Uuid,
        start: NaiveDate,
    ) -> AppResult<AnniversaryRow> {
        let (lo, hi) = if creator < other { (creator, other) } else { (other, creator) };
        Ok(sqlx::query_as(concat!(
            "INSERT INTO plan_anniversaries AS a (kind, user_lo, user_hi, created_by, start_date, status) VALUES ($1, $2, $3, $4, $5, $6) RETURNING ",
            anniversary_columns!()
        ))
        .bind(kind as i32 as i16)
        .bind(lo)
        .bind(hi)
        .bind(creator)
        .bind(start)
        .bind(status as i32 as i16)
        .fetch_one(conn)
        .await?)
    }

    /// Marks a counter as existing for real: schedules its first milestone
    /// notice and announces it to other domains.
    async fn start_counting(&self, conn: &mut PgConnection, row: &AnniversaryRow, actor: Uuid) -> AppResult<()> {
        let due = self.notice_due_on(conn, row).await?;
        sqlx::query("UPDATE plan_anniversaries SET notice_due_at = $2 WHERE id = $1")
            .bind(row.id)
            .bind(due)
            .execute(&mut *conn)
            .await?;
        let started =
            AnniversaryStarted { anniversary_id: row.id, kind: row.kind as i32, user_ids: [row.user_lo, row.user_hi] };
        bus::emit(&mut *conn, ANNIVERSARY_STARTED, Some(actor), &started).await
    }

    /// Starts a "days since we became friends" counter with a friend.
    pub(crate) async fn start_friendship_counter(
        &self,
        user_id: Uuid,
        friend: Uuid,
        start: &str,
    ) -> AppResult<Anniversary> {
        let core = &self.core;
        people::require_friends(core, user_id, &[friend]).await?;
        // Starting a counter tells the friend; starting and ending it over and over should not.
        core.cache.rate_limit(&format!("plans:counter:{user_id}"), 20, StdDuration::from_secs(24 * 3600)).await?;
        core.cache
            .rate_limit(
                &format!("plans:counter:{user_id}:{friend}"),
                COUNTERS_PER_FRIEND_PER_DAY,
                StdDuration::from_secs(24 * 3600),
            )
            .await?;
        let zone = dates::zone_of(core, user_id).await?;
        let today = dates::day_in(zone, now());
        let start = if start.is_empty() {
            // The day the friendship began, on the caller's calendar.
            let since = self.friendship(user_id, friend).await?.since;
            since.map(|since| dates::day_in(zone, since)).unwrap_or(today).min(today)
        } else {
            start_date(start, today)?
        };

        let mut tx = core.db.begin().await?;
        lock_pair(&mut tx, user_id, friend).await?;
        ensure!(
            !exists_between(&mut tx, AnniversaryKind::ANNIVERSARY_KIND_FRIENDSHIP, user_id, friend).await?,
            already_exists,
            "ANNIVERSARY_EXISTS",
            "the two of you already have this counter"
        );
        let row = self
            .insert_anniversary(
                &mut tx,
                AnniversaryKind::ANNIVERSARY_KIND_FRIENDSHIP,
                AnniversaryStatus::ANNIVERSARY_STATUS_ACTIVE,
                user_id,
                friend,
                start,
            )
            .await?;
        self.start_counting(&mut tx, &row, user_id).await?;
        let by = people::name_on(&mut tx, user_id).await?;
        let text = copy::counter_started(&by);
        let request =
            NotificationRequest::new(friend, "plans.counter_started", text.title, text.body, anniversary_path(row.id))
                .actor(user_id)
                .subject(SubjectType::SUBJECT_TYPE_USER, user_id)
                .inbox_only()
                .data(serde_json::json!({ "anniversary_id": row.id }));
        outbound::notify(&mut *tx, &request).await?;
        tx.commit().await?;

        self.push_anniversary(&row, friend).await?;
        self.anniversary_for(&row, user_id).await
    }

    /// Invites a friend to a couple counter.
    pub(crate) async fn invite_couple(&self, user_id: Uuid, friend: Uuid, start: &str) -> AppResult<Anniversary> {
        let core = &self.core;
        people::require_friends(core, user_id, &[friend]).await?;
        core.cache.rate_limit(&format!("plans:couple:invite:{user_id}"), 10, StdDuration::from_secs(24 * 3600)).await?;
        // Inviting, withdrawing and inviting again notifies the same person each time.
        core.cache
            .rate_limit(
                &format!("plans:couple:invite:{user_id}:{friend}"),
                PER_FRIEND_PER_DAY,
                StdDuration::from_secs(24 * 3600),
            )
            .await?;
        let today = dates::day_in(dates::zone_of(core, user_id).await?, now());
        let start = if start.is_empty() { today } else { start_date(start, today)? };

        let mut tx = core.db.begin().await?;
        lock_pair(&mut tx, user_id, friend).await?;
        ensure!(
            !in_couple(&mut tx, user_id).await?,
            precondition,
            "ALREADY_IN_COUPLE",
            "you already have a couple counter"
        );
        ensure!(
            !exists_between(&mut tx, AnniversaryKind::ANNIVERSARY_KIND_COUPLE, user_id, friend).await?,
            already_exists,
            "ANNIVERSARY_EXISTS",
            "an invitation between the two of you is already waiting"
        );
        // One invitation at a time: asking several people at once is not what this is for.
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM plan_anniversaries WHERE kind = 2 AND status = 1 AND created_by = $1)",
        )
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(!waiting, precondition, "INVITE_PENDING", "you already have an invitation waiting for an answer");
        let row = self
            .insert_anniversary(
                &mut tx,
                AnniversaryKind::ANNIVERSARY_KIND_COUPLE,
                AnniversaryStatus::ANNIVERSARY_STATUS_PENDING,
                user_id,
                friend,
                start,
            )
            .await?;
        let by = people::name_on(&mut tx, user_id).await?;
        let text = copy::couple_invite(&by);
        let request =
            NotificationRequest::new(friend, "plans.couple_invite", text.title, text.body, anniversary_path(row.id))
                .actor(user_id)
                .subject(SubjectType::SUBJECT_TYPE_USER, user_id)
                .collapse(format!("plans.couple_invite:{user_id}"))
                .data(serde_json::json!({ "anniversary_id": row.id }));
        outbound::notify(&mut *tx, &request).await?;
        tx.commit().await?;

        self.push_anniversary(&row, friend).await?;
        self.anniversary_for(&row, user_id).await
    }

    /// Accepts or declines a couple invitation addressed to `user_id`.
    pub(crate) async fn respond_couple_invite(
        &self,
        user_id: Uuid,
        anniversary_id: Uuid,
        accept: bool,
    ) -> AppResult<Option<Anniversary>> {
        let mut tx = self.core.db.begin().await?;
        let invitation = self.anniversary_of(&mut tx, anniversary_id, user_id, false).await?;
        // Only the invited person answers, and only while it is an invitation.
        ensure!(
            !invitation.is_active() && invitation.created_by != user_id,
            not_found,
            "ANNIVERSARY_NOT_FOUND",
            "invitation not found"
        );
        let inviter = invitation.created_by;
        lock_pair(&mut tx, user_id, inviter).await?;
        let invitation = self.anniversary_of(&mut tx, anniversary_id, user_id, true).await?;
        ensure!(!invitation.is_active(), not_found, "ANNIVERSARY_NOT_FOUND", "invitation not found");

        if !accept {
            // Quietly: the sender is not told.
            sqlx::query("DELETE FROM plan_anniversaries WHERE id = $1").bind(anniversary_id).execute(&mut *tx).await?;
            tx.commit().await?;
            return Ok(None);
        }
        ensure!(
            !in_couple(&mut tx, user_id).await?,
            precondition,
            "ALREADY_IN_COUPLE",
            "you already have a couple counter"
        );
        // The sender found someone else in the meantime. Their invitations
        // are withdrawn when that happens (below); if this one is still here
        // it is treated the same way — the answer must not tell the invited
        // person anything about the sender's other counters.
        if in_couple(&mut tx, inviter).await? {
            sqlx::query("DELETE FROM plan_anniversaries WHERE id = $1").bind(anniversary_id).execute(&mut *tx).await?;
            tx.commit().await?;
            return Err(not_found());
        }
        let row: AnniversaryRow = sqlx::query_as(concat!(
            "UPDATE plan_anniversaries a SET status = 2 WHERE a.id = $1 RETURNING ",
            anniversary_columns!()
        ))
        .bind(anniversary_id)
        .fetch_one(&mut *tx)
        .await?;
        self.start_counting(&mut tx, &row, user_id).await?;
        // The two are a couple now: whatever invitations either of them still
        // had out are withdrawn, as if they had taken them back themselves.
        let withdrawn: Vec<(Uuid, Uuid, Uuid, Uuid)> = sqlx::query_as(
            "DELETE FROM plan_anniversaries WHERE kind = 2 AND status = 1 AND created_by = ANY($1)
             RETURNING id, user_lo, user_hi, created_by",
        )
        .bind(vec![user_id, inviter])
        .fetch_all(&mut *tx)
        .await?;
        let by = people::name_on(&mut tx, user_id).await?;
        let text = copy::couple_accepted(&by);
        let request =
            NotificationRequest::new(inviter, "plans.couple_accepted", text.title, text.body, anniversary_path(row.id))
                .actor(user_id)
                .subject(SubjectType::SUBJECT_TYPE_USER, user_id)
                .data(serde_json::json!({ "anniversary_id": row.id }));
        outbound::notify(&mut *tx, &request).await?;
        tx.commit().await?;

        for (withdrawn_id, lo, hi, sender) in withdrawn {
            self.push([if lo == sender { hi } else { lo }], anniversary_ended(withdrawn_id)).await;
        }
        self.push_anniversary(&row, inviter).await?;
        Ok(Some(self.anniversary_for(&row, user_id).await?))
    }

    /// Changes a counter; either of the two may.
    pub(crate) async fn update_anniversary(
        &self,
        user_id: Uuid,
        req: &UpdateAnniversaryRequest,
    ) -> AppResult<Anniversary> {
        let core = &self.core;
        let anniversary_id = parse_id(&req.anniversary_id)?;
        // Every change is pushed to the other person.
        core.cache.rate_limit(&format!("plans:counter:update:{user_id}"), 120, StdDuration::from_secs(3600)).await?;
        let today = dates::day_in(dates::zone_of(core, user_id).await?, now());

        let mut tx = core.db.begin().await?;
        let mut row = self.anniversary_of(&mut tx, anniversary_id, user_id, true).await?;
        // An invitation is answered or withdrawn, not edited.
        ensure!(row.is_active(), not_found, "ANNIVERSARY_NOT_FOUND", "counter not found");
        let (old_start, old_counting) = (row.start_date, row.count_start_day);
        if let Some(start) = req.start_date.as_deref() {
            row.start_date = start_date(start, today)?;
        }
        if let Some(count_start_day) = req.count_start_day {
            row.count_start_day = count_start_day;
        }
        if let Some(title) = req.title.as_deref() {
            row.title = input::line(title, MAX_TITLE, "TITLE_TOO_LONG")?;
        }
        if let Some(emoji) = req.emoji.as_deref() {
            row.emoji = input::emoji(emoji)?;
        }
        if let Some(show) = req.show_on_profile {
            ensure!(row.is_couple(), invalid, "NOT_A_COUPLE", "only a couple counter can be shown on profiles");
            if row.user_lo == user_id {
                row.show_lo = show;
            } else {
                row.show_hi = show;
            }
        }
        let recount = (row.start_date, row.count_start_day) != (old_start, old_counting);
        if recount {
            // The milestones fall on other days now: what was announced no longer counts.
            sqlx::query("DELETE FROM plan_anniversary_notices WHERE anniversary_id = $1")
                .bind(row.id)
                .execute(&mut *tx)
                .await?;
        }
        let due = self.notice_due_on(&mut tx, &row).await?;
        sqlx::query(
            "UPDATE plan_anniversaries SET start_date = $2, count_start_day = $3, title = $4, emoji = $5, show_lo = $6, show_hi = $7,
                    notice_due_at = CASE WHEN $8 THEN $9 ELSE notice_due_at END
             WHERE id = $1",
        )
        .bind(row.id)
        .bind(row.start_date)
        .bind(row.count_start_day)
        .bind(&row.title)
        .bind(&row.emoji)
        .bind(row.show_lo)
        .bind(row.show_hi)
        .bind(recount)
        .bind(due)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        self.push_anniversary(&row, row.partner_of(user_id)).await?;
        self.anniversary_for(&row, user_id).await
    }

    /// Ends a counter, or withdraws / turns down an invitation.
    pub(crate) async fn end_anniversary(&self, user_id: Uuid, anniversary_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let row = self.anniversary_of(&mut tx, anniversary_id, user_id, true).await?;
        sqlx::query("DELETE FROM plan_anniversaries WHERE id = $1").bind(row.id).execute(&mut *tx).await?;
        tx.commit().await?;
        // Turning an invitation down stays quiet, like declining it.
        if row.is_active() || row.created_by == user_id {
            self.push([row.partner_of(user_id)], anniversary_ended(row.id)).await;
        }
        Ok(())
    }

    /// `user_id`'s counters and invitations, oldest start first.
    pub(crate) async fn anniversaries_of(&self, user_id: Uuid) -> AppResult<Vec<AnniversaryRow>> {
        let core = &self.core;
        // A block ends counters (see `maintenance`); until that event has been
        // handled the counter is simply not shown.
        let hidden = graph::blocked_ids_either(core, user_id).await?;
        Ok(sqlx::query_as(concat!(
            "SELECT ",
            anniversary_columns!(),
            " FROM plan_anniversaries a
              WHERE (a.user_lo = $1 AND NOT (a.user_hi = ANY($2))) OR (a.user_hi = $1 AND NOT (a.user_lo = ANY($2)))
              ORDER BY a.start_date, a.id"
        ))
        .bind(user_id)
        .bind(&hidden)
        .fetch_all(&core.db)
        .await?)
    }

    pub(crate) async fn list_anniversaries(&self, user_id: Uuid) -> AppResult<Vec<Anniversary>> {
        let core = &self.core;
        let rows = self.anniversaries_of(user_id).await?;
        let partners: Vec<Uuid> = rows.iter().map(|row| row.partner_of(user_id)).collect();
        let summaries = tokuly_identity::summaries(core, &partners).await?;
        let today = dates::day_in(dates::zone_of(core, user_id).await?, now());
        Ok(rows
            .iter()
            .map(|row| row.to_proto(user_id, tokuly_identity::users::pick(&summaries, row.partner_of(user_id)), today))
            .collect())
    }

    /// The couple counter `owner` is in, when both chose to show it.
    pub(crate) async fn couple_on_profile(&self, owner: Uuid) -> AppResult<Option<AnniversaryRow>> {
        let row: Option<AnniversaryRow> = sqlx::query_as(concat!(
            "SELECT ",
            anniversary_columns!(),
            " FROM plan_anniversaries a WHERE a.kind = 2 AND a.status = 2 AND a.show_lo AND a.show_hi AND (a.user_lo = $1 OR a.user_hi = $1)"
        ))
        .bind(owner)
        .fetch_optional(&self.core.db)
        .await?;
        Ok(row.filter(AnniversaryRow::on_profiles))
    }

    /// Announces today's milestones of the counters whose notice is due, to
    /// each of the two on their own morning, and schedules the next notice.
    /// Returns how many counters were handled.
    pub(crate) async fn announce_due_milestones(&self) -> AppResult<u64> {
        let mut tx = self.core.db.begin().await?;
        let due: Vec<AnniversaryRow> = sqlx::query_as(concat!(
            "SELECT ",
            anniversary_columns!(),
            " FROM plan_anniversaries a WHERE a.notice_due_at <= now() ORDER BY a.notice_due_at LIMIT 200 FOR UPDATE SKIP LOCKED"
        ))
        .fetch_all(&mut *tx)
        .await?;
        // Names and time zones are read on this transaction's connection.
        let people: Vec<Uuid> = due.iter().flat_map(|row| [row.user_lo, row.user_hi]).collect();
        let locals = people::locals(&mut tx, &people).await?;
        let now = now();
        for row in &due {
            let zone_of = |user_id: Uuid| locals.get(&user_id).map(|local| local.zone).unwrap_or(dates::FALLBACK_ZONE);
            for user_id in [row.user_lo, row.user_hi].into_iter().filter(|_| row.is_active()) {
                let zone = zone_of(user_id);
                let today = dates::day_in(zone, now);
                // Only on the day itself, from the morning hour on.
                let Some(milestone) = milestones::on(row.start_date, row.count_start_day, today)
                    .filter(|_| now >= dates::local_instant(today, self.cfg.morning_hour, zone))
                else {
                    continue;
                };
                let first = sqlx::query("INSERT INTO plan_anniversary_notices (anniversary_id, user_id, milestone) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING")
                    .bind(row.id)
                    .bind(user_id)
                    .bind(milestone.mark.key())
                    .execute(&mut *tx)
                    .await?
                    .rows_affected()
                    > 0;
                if !first {
                    continue;
                }
                let partner = row.partner_of(user_id);
                let partner_name = locals.get(&partner).map(|local| local.name.as_str()).unwrap_or("@deleted");
                let text = copy::milestone(partner_name, milestone.mark, row.is_couple());
                let request = NotificationRequest::new(user_id, "plans.milestone", text.title, text.body, anniversary_path(row.id))
                    .actor(partner)
                    .subject(SubjectType::SUBJECT_TYPE_USER, partner)
                    .collapse(format!("plans.milestone:{}:{}", row.id, milestone.mark.key()))
                    .data(serde_json::json!({ "anniversary_id": row.id, "milestone": milestone.mark.key(), "date": dates::format_date(milestone.date) }));
                outbound::notify(&mut *tx, &request).await?;
            }
            let next = notice_due(
                row.start_date,
                row.count_start_day,
                [zone_of(row.user_lo), zone_of(row.user_hi)],
                now,
                self.cfg.morning_hour,
            )
            .filter(|_| row.is_active());
            sqlx::query("UPDATE plan_anniversaries SET notice_due_at = $2 WHERE id = $1")
                .bind(row.id)
                .bind(next)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(due.len() as u64)
    }
}

/// The realtime event saying that a counter is gone.
pub(crate) fn anniversary_ended(anniversary_id: Uuid) -> Push {
    Push::AnniversaryChanged(Box::new(AnniversaryChanged {
        ended_anniversary_id: anniversary_id.to_string(),
        ..Default::default()
    }))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    const TOKYO: Tz = chrono_tz::Asia::Tokyo;
    const HONOLULU: Tz = chrono_tz::Pacific::Honolulu;

    fn date(text: &str) -> NaiveDate {
        dates::parse_date(text).unwrap()
    }

    fn at(zone: Tz, month: u32, day: u32, hour: u32) -> Time {
        zone.with_ymd_and_hms(2026, month, day, hour, 0, 0).unwrap().with_timezone(&chrono::Utc)
    }

    fn counter(start: &str, status: AnniversaryStatus) -> AnniversaryRow {
        AnniversaryRow {
            id: Uuid::nil(),
            kind: AnniversaryKind::ANNIVERSARY_KIND_COUPLE as i32 as i16,
            user_lo: Uuid::from_u128(1),
            user_hi: Uuid::from_u128(2),
            created_by: Uuid::from_u128(1),
            start_date: date(start),
            count_start_day: true,
            title: String::new(),
            emoji: String::new(),
            status: status as i32 as i16,
            show_lo: true,
            show_hi: false,
            created_at: at(TOKYO, 1, 1, 0),
        }
    }

    #[test]
    fn each_of_the_two_is_told_on_their_own_morning() {
        // Day 100 of a counter started on 2026-01-01 is 2026-04-10.
        let start = date("2026-01-01");
        // Tokyo reaches the morning of April 10 long before Honolulu does.
        let before = at(TOKYO, 4, 9, 12);
        assert_eq!(notice_due(start, true, [TOKYO, HONOLULU], before, 8), Some(at(TOKYO, 4, 10, 8)));
        // Once Tokyo's morning has come, Honolulu's is next.
        let tokyo_morning = at(TOKYO, 4, 10, 8);
        assert_eq!(notice_due(start, true, [TOKYO, HONOLULU], tokyo_morning, 8), Some(at(HONOLULU, 4, 10, 8)));
        // After both, the next milestone: half a year, on July 1.
        let after = at(HONOLULU, 4, 10, 9);
        assert_eq!(notice_due(start, true, [TOKYO, HONOLULU], after, 8), Some(at(TOKYO, 7, 1, 8)));
    }

    #[test]
    fn a_counter_shows_its_days_and_what_comes_next() {
        let row = counter("2026-01-01", AnniversaryStatus::ANNIVERSARY_STATUS_ACTIVE);
        let me = Uuid::from_u128(1);
        let shown = row.to_proto(me, UserSummary::default(), date("2026-04-09"));
        assert_eq!(shown.days, 99);
        assert_eq!(shown.upcoming.len(), 3);
        assert_eq!(
            (shown.upcoming[0].date.as_str(), shown.upcoming[0].days_left, shown.upcoming[0].number),
            ("2026-04-10", 1, 100)
        );
        assert_eq!(shown.upcoming[0].label.as_option().unwrap().ja, "100日記念日");
        // Each side has their own profile choice; it is on profiles only when both agree.
        assert!(shown.show_on_profile && !shown.partner_shows_on_profile);
        assert!(!row.on_profiles());
        let partner_view = row.to_proto(Uuid::from_u128(2), UserSummary::default(), date("2026-04-09"));
        assert!(!partner_view.show_on_profile && partner_view.partner_shows_on_profile);
        assert_eq!(row.partner_of(me), Uuid::from_u128(2));
    }

    #[test]
    fn an_invitation_counts_nothing_yet() {
        let invitation = counter("2026-01-01", AnniversaryStatus::ANNIVERSARY_STATUS_PENDING);
        let sent = invitation.to_proto(Uuid::from_u128(1), UserSummary::default(), date("2026-04-09"));
        assert_eq!((sent.days, sent.upcoming.len(), sent.invited_by_me), (0, 0, true));
        let received = invitation.to_proto(Uuid::from_u128(2), UserSummary::default(), date("2026-04-09"));
        assert!(!received.invited_by_me);
        assert!(!invitation.on_profiles());
    }

    #[test]
    fn start_dates_are_real_days_up_to_today() {
        let today = date("2026-11-22");
        assert_eq!(start_date("2026-11-22", today).unwrap(), today);
        assert_eq!(start_date("2020-02-29", today).unwrap(), date("2020-02-29"));
        assert_eq!(start_date("2026-11-23", today).unwrap_err().reason, "DATE_INVALID");
        assert_eq!(start_date("someday", today).unwrap_err().reason, "DATE_INVALID");
    }
}
