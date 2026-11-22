//! The study log: logging time by hand, correcting and deleting sessions,
//! and listing them.
use chrono::Duration;
use sqlx::{PgConnection, Postgres, Transaction};
use tokuly_core::prelude::*;
use tokuly_proto::{
    common::PageInfo,
    school::{
        ListStudySessionsRequest, LogStudySessionRequest, StudySession, UpdateStudySessionRequest,
        school_event::Event as Push,
    },
};

use super::sessions::{self, ENDED, SOURCE_MANUAL, SessionRow};
use crate::{School, local, subjects, text};

const MIN_LOGGED_SECONDS: i64 = 60;
const MAX_LOGGED_HOURS: i64 = 24;
const MAX_AGE_DAYS: i64 = 730;

fn not_found() -> AppError {
    AppError::not_found("SESSION_NOT_FOUND", "study session not found")
}

/// A stretch of study time entered by hand must lie in the past, be between
/// a minute and a day long, and not be ancient.
fn check_range(start: Time, end: Time, at: Time) -> AppResult<()> {
    let length = end - start;
    ensure!(
        length >= Duration::seconds(MIN_LOGGED_SECONDS) && length <= Duration::hours(MAX_LOGGED_HOURS),
        invalid,
        "TIME_RANGE_INVALID",
        "a session lasts between a minute and {MAX_LOGGED_HOURS} hours"
    );
    ensure!(end <= at, invalid, "TIME_RANGE_INVALID", "study time can only be logged once it is over");
    ensure!(
        start >= at - Duration::days(MAX_AGE_DAYS),
        invalid,
        "TIME_RANGE_INVALID",
        "study time can be logged up to two years back"
    );
    Ok(())
}

/// Fails when the user already has study time between two instants (in
/// sessions other than `except`), or a session going that started before `end`.
async fn check_free(
    conn: &mut PgConnection,
    user_id: Uuid,
    start: Time,
    end: Time,
    except: Option<Uuid>,
) -> AppResult<()> {
    let taken: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT EXISTS(SELECT 1 FROM school_study_segments
                       WHERE user_id = $1 AND started_at < $3 AND ended_at > $2 AND session_id IS DISTINCT FROM $4)
             OR EXISTS(SELECT 1 FROM school_study_sessions WHERE user_id = $1 AND state <> {ENDED} AND started_at < $3)"
    )))
    .bind(user_id)
    .bind(start)
    .bind(end)
    .bind(except)
    .fetch_one(conn)
    .await?;
    ensure!(!taken, already_exists, "SESSION_OVERLAPS", "study time is already logged in that window");
    Ok(())
}

/// Makes a finished session one uninterrupted stretch between two instants.
async fn set_stretch(
    tx: &mut Transaction<'_, Postgres>,
    session: &mut SessionRow,
    start: Time,
    end: Time,
) -> AppResult<()> {
    sqlx::query("DELETE FROM school_study_segments WHERE session_id = $1").bind(session.id).execute(&mut **tx).await?;
    sqlx::query(
        "INSERT INTO school_study_segments (session_id, user_id, started_at, ended_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(session.id)
    .bind(session.user_id)
    .bind(start)
    .bind(end)
    .execute(&mut **tx)
    .await?;
    session.started_at = start;
    session.ended_at = Some(end);
    session.checkpoint_at = end;
    session.heartbeat_at = end;
    session.counted_seconds = (end - start).num_seconds() as i32;
    sqlx::query("UPDATE school_study_sessions SET started_at = $2, ended_at = $3, checkpoint_at = $3, heartbeat_at = $3, counted_seconds = $4 WHERE id = $1")
        .bind(session.id)
        .bind(start)
        .bind(end)
        .bind(session.counted_seconds)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

impl School {
    pub(crate) async fn log_session(&self, user_id: Uuid, request: &LogStudySessionRequest) -> AppResult<StudySession> {
        let core = &self.core;
        let at = self.now();
        let (Some(start), Some(end)) =
            (request.started_at.as_option().map(from_ts), request.ended_at.as_option().map(from_ts))
        else {
            return Err(AppError::invalid("TIME_RANGE_INVALID", "say when the session started and ended"));
        };
        check_range(start, end, at)?;
        let memo = text::memo(&request.memo, "memo")?;
        let subject = subjects::require_opt(core, user_id, &request.subject_id).await?;

        let mut tx = core.db.begin().await?;
        crate::lock_user(&mut tx, "session", user_id).await?;
        check_free(&mut tx, user_id, start, end, None).await?;
        let mut session: SessionRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO school_study_sessions (user_id, subject_id, memo, source, state, started_at, ended_at, checkpoint_at, heartbeat_at)
             VALUES ($1, $2, $3, {SOURCE_MANUAL}, {ENDED}, $4, $5, $5, $5) RETURNING {}",
            sessions::COLUMNS
        )))
        .bind(user_id)
        .bind(subject.as_ref().map(|s| s.id))
        .bind(&memo)
        .bind(start)
        .bind(end)
        .fetch_one(&mut *tx)
        .await?;
        set_stretch(&mut tx, &mut session, start, end).await?;
        self.announce_logged(&mut tx, &session, subject.as_ref()).await?;
        tx.commit().await?;
        self.track_logged(&session).await;
        Ok(session.to_proto(subject.as_ref(), &session.own_timer(), at, self.idle_limit()))
    }

    pub(crate) async fn update_session(
        &self,
        user_id: Uuid,
        request: &UpdateStudySessionRequest,
    ) -> AppResult<StudySession> {
        let core = &self.core;
        let at = self.now();
        let session_id = parse_id(&request.session_id)?;
        let memo = request.memo.as_deref().map(|memo| text::memo(memo, "memo")).transpose()?;
        // `Some(None)` clears the subject.
        let subject = match &request.subject_id {
            Some(id) => Some(subjects::require_opt(core, user_id, id).await?),
            None => None,
        };
        let stretch = match (request.started_at.as_option().map(from_ts), request.ended_at.as_option().map(from_ts)) {
            (Some(start), Some(end)) => {
                check_range(start, end, at)?;
                Some((start, end))
            }
            (None, None) => None,
            _ => return Err(AppError::invalid("TIME_RANGE_INVALID", "change the start and the end together")),
        };

        let mut tx = core.db.begin().await?;
        crate::lock_user(&mut tx, "session", user_id).await?;
        let session: Option<SessionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM school_study_sessions WHERE id = $1 AND user_id = $2 FOR UPDATE",
            sessions::COLUMNS
        )))
        .bind(session_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;
        let mut session = session.ok_or_else(not_found)?;
        if let Some(subject) = &subject {
            // What a member studies in a room is part of the room's presence.
            ensure!(
                !(session.is_active() && session.in_room()),
                precondition,
                "ROOM_SESSION",
                "change the subject with SetStudyRoomSubject while in a room"
            );
            session.subject_id = subject.as_ref().map(|s| s.id);
        }
        if let Some(memo) = memo {
            session.memo = memo;
        }
        sqlx::query("UPDATE school_study_sessions SET subject_id = $2, memo = $3 WHERE id = $1")
            .bind(session.id)
            .bind(session.subject_id)
            .bind(&session.memo)
            .execute(&mut *tx)
            .await?;
        if let Some((start, end)) = stretch {
            ensure!(
                !session.is_active(),
                precondition,
                "SESSION_ACTIVE",
                "the times of a session can be changed once it has ended"
            );
            check_free(&mut tx, user_id, start, end, Some(session.id)).await?;
            set_stretch(&mut tx, &mut session, start, end).await?;
        }
        tx.commit().await?;

        let proto = self.session_proto(&session, None, at).await?;
        if session.is_active() {
            // Other devices show the timer too, and friends see the subject.
            self.push(user_id, Push::Session(Box::new(proto.clone()))).await;
            self.announce_studying_quietly(user_id, at).await;
        }
        Ok(proto)
    }

    pub(crate) async fn delete_session(&self, user_id: Uuid, session_id: Uuid) -> AppResult<()> {
        let state: Option<i16> =
            sqlx::query_scalar("SELECT state FROM school_study_sessions WHERE id = $1 AND user_id = $2")
                .bind(session_id)
                .bind(user_id)
                .fetch_optional(&self.core.db)
                .await?;
        let state = state.ok_or_else(not_found)?;
        ensure!(state == ENDED, precondition, "SESSION_ACTIVE", "stop the session before deleting it");
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM school_study_sessions WHERE id = $1 AND user_id = $2 AND state = {ENDED}"
        )))
        .bind(session_id)
        .bind(user_id)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    /// The caller's finished sessions, newest first.
    pub(crate) async fn list_sessions(
        &self,
        user_id: Uuid,
        request: &ListStudySessionsRequest,
        page: &Page,
    ) -> AppResult<(Vec<StudySession>, PageInfo)> {
        let core = &self.core;
        let at = self.now();
        let subject_id = parse_opt_id(&request.subject_id)?;
        let tz = local::time_zone(core, user_id).await?;
        let from = local::parse_opt_date(&request.from_date)?.map(|date| local::day_start(date, tz));
        let to = local::parse_opt_date(&request.to_date)?.map(|date| local::day_start(date + Duration::days(1), tz));
        let cursor: Option<(Time, Uuid)> = page.decode()?;
        let mut rows: Vec<SessionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM school_study_sessions
             WHERE user_id = $1 AND state = {ENDED}
               AND ($2::uuid IS NULL OR subject_id = $2)
               AND ($3::timestamptz IS NULL OR started_at >= $3)
               AND ($4::timestamptz IS NULL OR started_at < $4)
               AND ($5::timestamptz IS NULL OR (started_at, id) < ($5, $6))
             ORDER BY started_at DESC, id DESC LIMIT $7",
            sessions::COLUMNS
        )))
        .bind(user_id)
        .bind(subject_id)
        .bind(from)
        .bind(to)
        .bind(cursor.map(|(started_at, _)| started_at))
        .bind(cursor.map(|(_, id)| id))
        .bind(page.fetch())
        .fetch_all(&core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.started_at, row.id));
        let subjects =
            subjects::by_ids(core, &rows.iter().filter_map(|row| row.subject_id).collect::<Vec<_>>()).await?;
        let sessions = rows
            .iter()
            .map(|row| {
                row.to_proto(row.subject_id.and_then(|id| subjects.get(&id)), &row.own_timer(), at, self.idle_limit())
            })
            .collect();
        Ok((sessions, info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logged_time_must_be_past_and_of_a_sane_length() {
        let at = chrono::Utc::now();
        let ago = |minutes: i64| at - Duration::minutes(minutes);
        assert!(check_range(ago(90), ago(30), at).is_ok());
        assert!(check_range(ago(24 * 60), at, at).is_ok(), "exactly a day, ending now");
        for (start, end) in [
            (ago(30), ago(90)),                            // backwards
            (ago(30), ago(30)),                            // empty
            (ago(10), ago(10) + Duration::seconds(30)),    // too short
            (ago(25 * 60), ago(10)),                       // longer than a day
            (ago(30), at + Duration::minutes(1)),          // not over yet
            (ago(731 * 24 * 60), ago(731 * 24 * 60 - 60)), // too long ago
        ] {
            assert_eq!(check_range(start, end, at).unwrap_err().reason, "TIME_RANGE_INVALID");
        }
    }
}
