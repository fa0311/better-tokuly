//! Study sessions: the row, counting time into segments, and ending.
//!
//! A session counts study time in stretches (`school_study_segments`). Time
//! is counted — [`advance`] — whenever the client is heard from, and when a
//! study room's timer changes for everyone in it. A session the client ends
//! is counted up to that moment; a session the server has to end is cut back
//! to how far the client was believed ([`End::Server`]), so nobody gains time
//! by walking away from a running timer.
//!
//! The client is believed up to its last sign of life — or, when a pomodoro
//! phase was running then, up to that phase's planned end, because a phone
//! locked in the middle of a phase cannot send heartbeats. The server gives
//! up on a session the idle limit after that ([`School::is_overdue`]).
use chrono::{DateTime, Duration, Utc};
use sqlx::{PgConnection, Postgres, Transaction};
use tokuly_core::{
    analytics::AnalyticsEvent,
    events,
    outbound::{self, ActivityAudience, ActivityEntry, Localized},
    prelude::*,
};
use tokuly_proto::school::{SessionSource, SessionState, StudySession};

use super::stats;
use crate::{
    School,
    events::{GOAL_REACHED, GoalReached, SESSION_ENDED, SessionEnded},
    local,
    rooms::{self, RoomRow},
    settings,
    subjects::{self, SubjectRow},
    timer::{self, Plan, Run, Timer},
};

pub(crate) const RUNNING: i16 = SessionState::SESSION_STATE_RUNNING as i16;
pub(crate) const PAUSED: i16 = SessionState::SESSION_STATE_PAUSED as i16;
pub(crate) const ENDED: i16 = SessionState::SESSION_STATE_ENDED as i16;

pub(crate) const SOURCE_TIMER: i16 = SessionSource::SESSION_SOURCE_TIMER as i16;
pub(crate) const SOURCE_ROOM: i16 = SessionSource::SESSION_SOURCE_ROOM as i16;
pub(crate) const SOURCE_MANUAL: i16 = SessionSource::SESSION_SOURCE_MANUAL as i16;

/// Sessions shorter than this are logged but not announced to anyone.
const EVENT_MIN_SECONDS: i32 = 60;
/// Sessions shorter than this do not appear in friends' activity feeds.
const ACTIVITY_MIN_SECONDS: i32 = 600;

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct SessionRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub subject_id: Option<Uuid>,
    pub memo: String,
    pub source: i16,
    pub state: i16,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub mode: i16,
    pub work_seconds: i32,
    pub break_seconds: i32,
    pub long_break_seconds: i32,
    pub rounds: i32,
    pub timer_anchor_at: Option<DateTime<Utc>>,
    pub timer_offset_ms: i64,
    pub checkpoint_at: DateTime<Utc>,
    pub believed_until: DateTime<Utc>,
    pub paused_at: Option<DateTime<Utc>>,
    pub counted_seconds: i32,
    pub auto_stopped: bool,
    pub cheer_count: i32,
    pub room_id: Option<Uuid>,
    pub room_joined_at: Option<DateTime<Utc>>,
}

pub(crate) const COLUMNS: &str = "id, user_id, subject_id, memo, source, state, started_at, ended_at, mode, work_seconds, break_seconds, \
     long_break_seconds, rounds, timer_anchor_at, timer_offset_ms, checkpoint_at, believed_until, paused_at, counted_seconds, auto_stopped, \
     cheer_count, room_id, room_joined_at";

impl SessionRow {
    pub fn is_active(&self) -> bool {
        self.state != ENDED
    }

    pub fn in_room(&self) -> bool {
        self.source == SOURCE_ROOM
    }

    /// The session's own timer. (A session in a room follows the room's.)
    pub fn own_timer(&self) -> Timer {
        Timer {
            plan: Plan::from_columns(
                self.mode,
                self.work_seconds,
                self.break_seconds,
                self.long_break_seconds,
                self.rounds,
            ),
            run: match self.state {
                RUNNING => Run::Running,
                PAUSED => Run::Paused,
                _ => Run::Idle,
            },
            anchor: self.timer_anchor_at,
            offset_ms: self.timer_offset_ms,
        }
    }

    /// The timer the session counts by: the room's when it is in one.
    pub fn timer(&self, room: Option<&RoomRow>) -> Timer {
        match room.filter(|_| self.in_room()) {
            Some(room) => room.timer(),
            None => self.own_timer(),
        }
    }

    /// Study time at `at`: what is counted plus the work since, as far as
    /// the client is still believed (the idle limit has not caught up).
    pub fn studied_seconds(&self, timer: &Timer, at: Time, idle: Duration) -> i32 {
        if !self.is_active() {
            return self.counted_seconds;
        }
        let believed_until = at.min(self.believed_until + idle);
        self.counted_seconds
            + (timer::total_ms(&timer.work_intervals(self.checkpoint_at, believed_until)) / 1000) as i32
    }

    /// The session as its owner sees it. `timer` is the one it counts by.
    pub fn to_proto(&self, subject: Option<&SubjectRow>, timer: &Timer, at: Time, idle: Duration) -> StudySession {
        StudySession {
            id: self.id.to_string(),
            state: (self.state as i32).into(),
            source: (self.source as i32).into(),
            subject: subject.map(SubjectRow::to_proto).into(),
            memo: self.memo.clone(),
            started_at: ts(self.started_at),
            ended_at: opt_ts(self.ended_at),
            studied_seconds: self.studied_seconds(timer, at, idle),
            timer: self.is_active().then(|| timer.snapshot(at)).into(),
            room_id: self.room_id.map(|id| id.to_string()).unwrap_or_default(),
            auto_stopped: self.auto_stopped,
            cheer_count: self.cheer_count,
            ..Default::default()
        }
    }
}

/// A session that just ended, with what the caller needs to tell people.
/// (One that counted no study time is gone from the log altogether.)
#[derive(Debug, Clone)]
pub(crate) struct Ended {
    pub session: SessionRow,
    pub subject: Option<SubjectRow>,
}

/// How a session ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum End {
    /// The user stopped it (or left the room) at this moment.
    Client(Time),
    /// The server ends it: heartbeats stopped, a limit was reached, the room
    /// closed, the host removed the member — at this moment. Only the time
    /// up to how far the client was believed counts.
    Server(Time),
}

/// A new session to insert.
pub(crate) struct NewSession<'a> {
    pub user_id: Uuid,
    pub subject_id: Option<Uuid>,
    pub memo: &'a str,
    pub source: i16,
    /// The session's own timer (ignored in a room).
    pub plan: Plan,
    pub room: Option<(Uuid, Time)>,
    pub at: Time,
}

/// Inserts a running session started at `new.at`. `SESSION_ALREADY_ACTIVE`
/// when the user has one (the unique index decides, whatever raced).
pub(crate) async fn insert(conn: &mut PgConnection, new: &NewSession<'_>) -> AppResult<SessionRow> {
    let (work, short_break, long_break, rounds) = new.plan.columns();
    // A session in a room is believed further from its first heartbeat on.
    let believed = match new.room {
        None => believed_until(&Timer::started(new.plan, new.at), new.at),
        Some(_) => new.at,
    };
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "INSERT INTO school_study_sessions (user_id, subject_id, memo, source, state, started_at, mode, work_seconds, break_seconds,
             long_break_seconds, rounds, timer_anchor_at, checkpoint_at, believed_until, room_id, room_joined_at)
         VALUES ($1, $2, $3, $4, {RUNNING}, $5, $6, $7, $8, $9, $10, $11, $5, $14, $12, $13)
         RETURNING {COLUMNS}"
    )))
    .bind(new.user_id)
    .bind(new.subject_id)
    .bind(new.memo)
    .bind(new.source)
    .bind(new.at)
    .bind(new.plan.mode() as i16)
    .bind(work)
    .bind(short_break)
    .bind(long_break)
    .bind(rounds)
    // Only a session with its own timer has an anchor.
    .bind(new.room.is_none().then_some(new.at))
    .bind(new.room.map(|(room_id, _)| room_id))
    .bind(new.room.map(|(_, joined_at)| joined_at))
    .bind(believed)
    .fetch_one(conn)
    .await
    .map_err(|error| match &error {
        sqlx::Error::Database(db) if db.is_unique_violation() => already_active(),
        _ => error.into(),
    })
}

pub(crate) fn already_active() -> AppError {
    AppError::precondition("SESSION_ALREADY_ACTIVE", "you already have a study session going")
}

pub(crate) fn no_active_session() -> AppError {
    AppError::precondition("NO_ACTIVE_SESSION", "you have no study session going")
}

/// Sets `counted_seconds` to the sum of the session's segments.
async fn recount(conn: &mut PgConnection, session: &mut SessionRow) -> AppResult<()> {
    session.counted_seconds = sqlx::query_scalar(
        "UPDATE school_study_sessions SET counted_seconds =
             (SELECT COALESCE(floor(sum(EXTRACT(EPOCH FROM ended_at - started_at))), 0)::int FROM school_study_segments WHERE session_id = $1)
         WHERE id = $1 RETURNING counted_seconds",
    )
    .bind(session.id)
    .fetch_one(conn)
    .await?;
    Ok(())
}

/// Counts the study time between the session's checkpoint and `at`, as the
/// timer it follows says: the work phases in between become segments (the
/// last segment grows when the work simply went on).
pub(crate) async fn advance(
    conn: &mut PgConnection,
    session: &mut SessionRow,
    timer: &Timer,
    at: Time,
) -> AppResult<()> {
    if at <= session.checkpoint_at {
        return Ok(());
    }
    for (start, end) in timer.work_intervals(session.checkpoint_at, at) {
        let grown =
            sqlx::query("UPDATE school_study_segments SET ended_at = $3 WHERE session_id = $1 AND ended_at = $2")
                .bind(session.id)
                .bind(start)
                .bind(end)
                .execute(&mut *conn)
                .await?;
        if grown.rows_affected() == 0 {
            sqlx::query(
                "INSERT INTO school_study_segments (session_id, user_id, started_at, ended_at) VALUES ($1, $2, $3, $4)",
            )
            .bind(session.id)
            .bind(session.user_id)
            .bind(start)
            .bind(end)
            .execute(&mut *conn)
            .await?;
        }
    }
    session.checkpoint_at = at;
    sqlx::query("UPDATE school_study_sessions SET checkpoint_at = $2 WHERE id = $1")
        .bind(session.id)
        .bind(at)
        .execute(&mut *conn)
        .await?;
    recount(conn, session).await
}

/// How far a client heard from at `at` is believed, with `timer` as it then
/// was: to the end of the pomodoro phase in progress, else just to `at`.
fn believed_until(timer: &Timer, at: Time) -> Time {
    timer.phase_ends_at(at).map_or(at, |end| end.max(at))
}

/// Notes in `session` that the client was heard from at `at`.
pub(crate) fn hear_from(session: &mut SessionRow, timer: &Timer, at: Time) {
    session.believed_until = believed_until(timer, at);
}

/// Records that the client was heard from at `at`.
pub(crate) async fn heard_from(
    conn: &mut PgConnection,
    session: &mut SessionRow,
    timer: &Timer,
    at: Time,
) -> AppResult<()> {
    hear_from(session, timer, at);
    sqlx::query("UPDATE school_study_sessions SET believed_until = $2 WHERE id = $1")
        .bind(session.id)
        .bind(session.believed_until)
        .execute(conn)
        .await?;
    Ok(())
}

/// Forgets the study time counted after `at`.
async fn cut_back(conn: &mut PgConnection, session: &mut SessionRow, at: Time) -> AppResult<()> {
    sqlx::query("DELETE FROM school_study_segments WHERE session_id = $1 AND started_at >= $2")
        .bind(session.id)
        .bind(at)
        .execute(&mut *conn)
        .await?;
    sqlx::query("UPDATE school_study_segments SET ended_at = $2 WHERE session_id = $1 AND ended_at > $2")
        .bind(session.id)
        .bind(at)
        .execute(&mut *conn)
        .await?;
    session.checkpoint_at = session.checkpoint_at.min(at);
    Ok(())
}

impl School {
    pub(crate) fn idle_limit(&self) -> Duration {
        Duration::seconds(self.cfg.session_idle_seconds)
    }

    /// Whether the server's limits have caught up with an active session:
    /// its client stayed silent for the idle limit past how far it was
    /// believed, it stayed paused too long, or it has lasted
    /// as long as a session may.
    pub(crate) fn is_overdue(&self, session: &SessionRow, at: Time) -> bool {
        if at >= session.started_at + Duration::seconds(self.cfg.session_max_seconds) {
            return true;
        }
        match (session.state, session.paused_at) {
            (PAUSED, Some(paused_at)) => at - paused_at > Duration::seconds(self.cfg.session_max_pause_seconds),
            _ => at - session.believed_until > self.idle_limit(),
        }
    }

    /// Ends an active session (locked by the caller) and logs it: the
    /// `school.study.session_ended` event, the activity entry and the goal
    /// event go out with the same transaction. A session that counted
    /// nothing is deleted instead.
    pub(crate) async fn finish(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        mut session: SessionRow,
        timer: &Timer,
        end: End,
    ) -> AppResult<Ended> {
        let ended_at = match end {
            End::Client(at) => {
                advance(tx, &mut session, timer, at).await?;
                at
            }
            End::Server(at) => {
                // A paused session stopped counting when it was paused; a
                // running one counts as far as it was believed, never past
                // now, nor past the longest a session may last.
                let longest = session.started_at + Duration::seconds(self.cfg.session_max_seconds);
                let until = match (session.state, session.paused_at) {
                    (PAUSED, Some(paused_at)) => paused_at,
                    _ => session.believed_until.min(at).min(longest),
                };
                // (A pomodoro phase believed to its end may not be counted yet.)
                advance(tx, &mut session, timer, until).await?;
                cut_back(tx, &mut session, until).await?;
                recount(tx, &mut session).await?;
                until
            }
        };
        session.state = ENDED;
        session.ended_at = Some(ended_at);
        session.paused_at = None;
        session.timer_anchor_at = None;
        session.auto_stopped = matches!(end, End::Server(_));
        let subject = match session.subject_id {
            Some(id) => subjects::require(tx, session.user_id, id).await.ok(),
            None => None,
        };
        if session.counted_seconds == 0 {
            sqlx::query("DELETE FROM school_study_sessions WHERE id = $1").bind(session.id).execute(&mut **tx).await?;
            return Ok(Ended { session, subject });
        }
        sqlx::query(
            "UPDATE school_study_sessions SET state = $2, ended_at = $3, paused_at = NULL, timer_anchor_at = NULL, auto_stopped = $4
             WHERE id = $1",
        )
        .bind(session.id)
        .bind(ENDED)
        .bind(ended_at)
        .bind(session.auto_stopped)
        .execute(&mut **tx)
        .await?;
        self.announce_logged(tx, &session, subject.as_ref()).await?;
        Ok(Ended { session, subject })
    }

    /// What follows from a session entering the log, in the transaction that
    /// logs it: the domain event, the friends' activity entry, the goal.
    pub(crate) async fn announce_logged(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        session: &SessionRow,
        subject: Option<&SubjectRow>,
    ) -> AppResult<()> {
        if session.counted_seconds < EVENT_MIN_SECONDS {
            return Ok(());
        }
        let user_id = session.user_id;
        let minutes = session.counted_seconds / 60;
        let ended_at = session.ended_at.unwrap_or(session.checkpoint_at);
        let manual = session.source == SOURCE_MANUAL;
        events::emit(
            &mut **tx,
            SESSION_ENDED,
            Some(user_id),
            &SessionEnded {
                user_id,
                session_id: session.id,
                minutes,
                seconds: session.counted_seconds,
                subject: subject.map(|s| s.name.clone()).unwrap_or_default(),
                subject_id: session.subject_id,
                source: session.source as i32,
                room_id: session.room_id,
                started_at: session.started_at,
                ended_at,
                manual,
                auto_stopped: session.auto_stopped,
            },
        )
        .await?;

        let settings = settings::of_on(tx, user_id).await?;
        // Someone who hides their studying does not have it announced in
        // feeds either, whatever the feed switch says.
        if !manual
            && session.counted_seconds >= ACTIVITY_MIN_SECONDS
            && settings.share_study_activity
            && settings.shows_studying()
            && shares_activity(tx, user_id).await?
        {
            let summary = match subject {
                Some(subject) => Localized::new(
                    format!("{}を{minutes}分勉強しました", subject.name),
                    format!("Studied {} for {minutes} min", subject.name),
                ),
                None => Localized::new(format!("{minutes}分勉強しました"), format!("Studied for {minutes} min")),
            };
            let entry = ActivityEntry::new(user_id, "school.studied", ActivityAudience::Friends, summary)
                .path("/school/study")
                .data(serde_json::json!({
                    "session_id": session.id,
                    "minutes": minutes,
                    "subject": subject.map(|s| s.name.as_str()),
                    // Which room is nobody's business in a feed: friends
                    // who may enter it see it in the list of who is studying.
                    "in_room": session.room_id.is_some(),
                }));
            outbound::activity(&mut **tx, &entry).await?;
        }

        // Did this session carry the day past the goal?
        if settings.daily_goal_minutes > 0 {
            let tz = local::time_zone_on(tx, user_id).await?;
            let day = local::local_date(ended_at, tz);
            let (day_from, day_to) = (local::day_start(day, tz), local::day_start(day + Duration::days(1), tz));
            let of_session: i64 = sqlx::query_scalar(
                "SELECT COALESCE(floor(sum(EXTRACT(EPOCH FROM LEAST(ended_at, $3) - GREATEST(started_at, $2)))), 0)::bigint
                 FROM school_study_segments WHERE session_id = $1 AND started_at < $3 AND ended_at > $2",
            )
            .bind(session.id)
            .bind(day_from)
            .bind(day_to)
            .fetch_one(&mut **tx)
            .await?;
            let progress = stats::progress(tx, user_id, tz, &settings, ended_at).await?;
            let goal = settings.daily_goal_minutes as i64 * 60;
            let today = progress.today_seconds as i64;
            if progress.date == local::fmt_date(day) && today >= goal && today - of_session < goal {
                let reached = GoalReached {
                    user_id,
                    date: progress.date.clone(),
                    minutes: (today / 60) as i32,
                    goal_minutes: settings.daily_goal_minutes,
                    streak_days: progress.streak_days,
                };
                events::emit(&mut **tx, GOAL_REACHED, Some(user_id), &reached).await?;
            }
        }
        Ok(())
    }

    /// Product analytics for a session that entered the log (`school.study.session`,
    /// valued in seconds). Called after the commit; tracking never fails.
    pub(crate) async fn track_logged(&self, session: &SessionRow) {
        if session.counted_seconds < EVENT_MIN_SECONDS {
            return;
        }
        let mut event = AnalyticsEvent::new("school.study.session")
            .user(Some(session.user_id))
            .subject("study_session", session.id)
            .value(session.counted_seconds as f64)
            .props(serde_json::json!({ "source": session.source, "room_id": session.room_id, "auto_stopped": session.auto_stopped }));
        event.at = session.ended_at.unwrap_or(session.checkpoint_at);
        self.core.analytics.track(event).await;
    }

    /// The caller's active session with the room it is in, both locked for
    /// the rest of the transaction (the room first: that is the order every
    /// room operation takes its locks in).
    pub(crate) async fn lock_active(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user_id: Uuid,
    ) -> AppResult<Option<(SessionRow, Option<RoomRow>)>> {
        // The session can change rooms between the peek and the lock; look again when it did.
        for _ in 0..3 {
            let peek: Option<(Uuid, Option<Uuid>)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT id, room_id FROM school_study_sessions WHERE user_id = $1 AND state <> {ENDED}"
            )))
            .bind(user_id)
            .fetch_optional(&mut **tx)
            .await?;
            let Some((session_id, room_id)) = peek else { return Ok(None) };
            let room = match room_id {
                Some(room_id) => rooms::lock(tx, room_id).await?,
                None => None,
            };
            let session: Option<SessionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {COLUMNS} FROM school_study_sessions WHERE id = $1 AND state <> {ENDED} FOR UPDATE"
            )))
            .bind(session_id)
            .fetch_optional(&mut **tx)
            .await?;
            match session {
                Some(session) if session.room_id == room_id => return Ok(Some((session, room))),
                _ => continue,
            }
        }
        Err(AppError::unavailable("SESSION_BUSY", "the session is changing; try again"))
    }
}

/// The account-wide switch for activity sharing (identity's privacy settings).
async fn shares_activity(conn: &mut PgConnection, user_id: Uuid) -> AppResult<bool> {
    let privacy: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT privacy FROM users WHERE id = $1").bind(user_id).fetch_optional(conn).await?;
    Ok(privacy.is_some_and(|privacy| tokuly_identity::settings::privacy_from_json(&privacy).share_activity))
}
