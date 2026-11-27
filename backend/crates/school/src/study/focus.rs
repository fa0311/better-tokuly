//! The focus timer: starting, pausing, resuming, skipping and stopping the
//! caller's session, and the heartbeat that keeps it counting.
use std::time::Duration as StdDuration;

use sqlx::{PgConnection, Postgres, Transaction};
use tokuly_core::prelude::*;
use tokuly_proto::school::{
    PomodoroConfig, StartStudySessionRequest, StudyMode, StudySession, school_event::Event as Push,
};

use super::sessions::{self, ENDED, End, Ended, NewSession, PAUSED, RUNNING, SOURCE_TIMER, SessionRow};
use crate::{
    School,
    rooms::{self, RoomExit, RoomRow},
    settings, subjects, text,
    timer::{Cycle, Plan, Run, Timer},
};

/// A session that is over, and the room it left with it.
pub(crate) struct Gone {
    pub ended: Ended,
    pub exit: Option<RoomExit>,
}

/// What the caller's active session turned out to be once the server's
/// limits were applied.
pub(crate) enum Active {
    None,
    Live(SessionRow, Option<RoomRow>),
    /// It had run past a limit and was ended just now (in the open transaction).
    Expired(Gone),
}

fn room_session() -> AppError {
    AppError::precondition("ROOM_SESSION", "in a study room the host controls the timer")
}

/// Writes a session's own timer back, with the state that follows from it.
async fn store_timer(conn: &mut PgConnection, session: &mut SessionRow, timer: &Timer, at: Time) -> AppResult<()> {
    session.state = if timer.run == Run::Running { RUNNING } else { PAUSED };
    session.paused_at = (session.state == PAUSED).then_some(at);
    session.timer_anchor_at = timer.anchor;
    session.timer_offset_ms = timer.offset_ms;
    sessions::hear_from(session, timer, at);
    // Nothing between the last count and now is owed: counting restarts here.
    session.checkpoint_at = at;
    sqlx::query(
        "UPDATE school_study_sessions SET state = $2, paused_at = $3, timer_anchor_at = $4, timer_offset_ms = $5, believed_until = $7, checkpoint_at = $6
         WHERE id = $1",
    )
    .bind(session.id)
    .bind(session.state)
    .bind(session.paused_at)
    .bind(session.timer_anchor_at)
    .bind(session.timer_offset_ms)
    .bind(at)
    .bind(session.believed_until)
    .execute(conn)
    .await?;
    Ok(())
}

impl School {
    /// The plan a request asks for: a stopwatch, or a pomodoro with the
    /// given lengths (the user's defaults for what is left out).
    pub(crate) fn plan(mode: StudyMode, pomodoro: Option<&PomodoroConfig>, defaults: Cycle) -> AppResult<Plan> {
        match mode {
            StudyMode::STUDY_MODE_STOPWATCH => Ok(Plan::Open),
            StudyMode::STUDY_MODE_POMODORO => {
                Ok(Plan::Pomodoro(Cycle::from_config(pomodoro.unwrap_or(&PomodoroConfig::default()), defaults)?))
            }
            StudyMode::STUDY_MODE_UNSPECIFIED => {
                Err(AppError::invalid("STUDY_MODE_REQUIRED", "choose a stopwatch or a pomodoro timer"))
            }
        }
    }

    /// The caller's active session, locked, after applying the limits.
    pub(crate) async fn active_session(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user_id: Uuid,
        at: Time,
    ) -> AppResult<Active> {
        Ok(match self.lock_active(tx, user_id).await? {
            None => Active::None,
            Some((session, room)) if self.is_overdue(&session, at) => {
                Active::Expired(self.end_locked(tx, session, room, End::Server(at), rooms::Leave::Idle, at).await?)
            }
            Some((session, room)) => Active::Live(session, room),
        })
    }

    /// Ends a locked active session; a session in a room leaves the room.
    pub(crate) async fn end_locked(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        session: SessionRow,
        room: Option<RoomRow>,
        end: End,
        leave: rooms::Leave,
        at: Time,
    ) -> AppResult<Gone> {
        match room.filter(|_| session.in_room()) {
            Some(mut room) => {
                let (ended, exit) = self.depart(tx, &mut room, session, end, leave, at).await?;
                Ok(Gone { ended, exit: Some(exit) })
            }
            None => {
                let timer = session.own_timer();
                Ok(Gone { ended: self.finish(tx, session, &timer, end).await?, exit: None })
            }
        }
    }

    /// Tells everyone concerned that a session is over (after the commit):
    /// the user's devices, their room, their friends.
    pub(crate) async fn announce_gone(&self, gone: &Gone, at: Time) {
        let session = &gone.ended.session;
        let proto = session.to_proto(gone.ended.subject.as_ref(), &session.own_timer(), at, self.idle_limit());
        if let Some(exit) = &gone.exit {
            self.announce_exit(session.user_id, exit, &proto, at).await;
        }
        self.push(session.user_id, Push::Session(Box::new(proto))).await;
        self.announce_studying_quietly(session.user_id, at).await;
        self.track_logged(session).await;
    }

    /// Tells the user's devices and friends about a change to a live session.
    async fn announce_session(&self, session: &SessionRow, at: Time) -> AppResult<StudySession> {
        let proto = self.session_proto(session, None, at).await?;
        self.push(session.user_id, Push::Session(Box::new(proto.clone()))).await;
        self.announce_studying_quietly(session.user_id, at).await;
        Ok(proto)
    }

    /// A session as its owner sees it. `room` is the room it is in, when the
    /// caller has it at hand.
    pub(crate) async fn session_proto(
        &self,
        session: &SessionRow,
        room: Option<&RoomRow>,
        at: Time,
    ) -> AppResult<StudySession> {
        let core = &self.core;
        let subject = match session.subject_id {
            Some(id) => subjects::by_ids(core, &[id]).await?.remove(&id),
            None => None,
        };
        let loaded;
        let room = match (room, session.room_id.filter(|_| session.is_active() && session.in_room())) {
            (Some(room), _) => Some(room),
            (None, Some(room_id)) => {
                loaded = rooms::find(core, room_id).await?;
                loaded.as_ref()
            }
            (None, None) => None,
        };
        Ok(session.to_proto(subject.as_ref(), &session.timer(room), at, self.idle_limit()))
    }

    pub(crate) async fn start_session(
        &self,
        user_id: Uuid,
        request: &StartStudySessionRequest,
    ) -> AppResult<StudySession> {
        let core = &self.core;
        let at = self.now();
        let mode = request.mode.as_known().unwrap_or(StudyMode::STUDY_MODE_UNSPECIFIED);
        let plan = Self::plan(mode, request.pomodoro.as_option(), settings::of(core, user_id).await?.pomodoro())?;
        let memo = text::memo(&request.memo, "memo")?;
        let subject = subjects::require_opt(core, user_id, &request.subject_id).await?;
        core.cache.rate_limit(&format!("school:start:{user_id}"), 30, StdDuration::from_secs(60)).await?;

        let mut tx = core.db.begin().await?;
        crate::lock_user(&mut tx, "session", user_id).await?;
        // A session the limits have caught up with makes room for the new one.
        let expired = match self.active_session(&mut tx, user_id, at).await? {
            Active::Live(..) => return Err(sessions::already_active()),
            Active::Expired(gone) => Some(gone),
            Active::None => None,
        };
        let new = NewSession {
            user_id,
            subject_id: subject.as_ref().map(|s| s.id),
            memo: &memo,
            source: SOURCE_TIMER,
            plan,
            room: None,
            at,
        };
        let session = sessions::insert(&mut tx, &new).await?;
        tx.commit().await?;
        if let Some(gone) = &expired {
            self.announce_gone(gone, at).await;
        }
        self.announce_session(&session, at).await
    }

    /// Runs `change` on the caller's own running or paused timer, with the
    /// time up to now counted first.
    async fn change_timer(
        &self,
        user_id: Uuid,
        at: Time,
        change: impl FnOnce(&SessionRow, &mut Timer) -> AppResult<()>,
    ) -> AppResult<StudySession> {
        // Every change is announced to the user's friends: not a button to hammer.
        self.core.cache.rate_limit(&format!("school:timer:{user_id}"), 30, StdDuration::from_secs(60)).await?;
        let mut tx = self.core.db.begin().await?;
        let mut session = match self.active_session(&mut tx, user_id, at).await? {
            Active::None => return Err(sessions::no_active_session()),
            Active::Expired(gone) => {
                tx.commit().await?;
                self.announce_gone(&gone, at).await;
                return Err(sessions::no_active_session());
            }
            Active::Live(session, _) if session.in_room() => return Err(room_session()),
            Active::Live(session, _) => session,
        };
        let mut timer = session.own_timer();
        sessions::advance(&mut tx, &mut session, &timer, at).await?;
        change(&session, &mut timer)?;
        store_timer(&mut tx, &mut session, &timer, at).await?;
        tx.commit().await?;
        self.announce_session(&session, at).await
    }

    pub(crate) async fn pause_session(&self, user_id: Uuid) -> AppResult<StudySession> {
        let at = self.now();
        self.change_timer(user_id, at, |session, timer| {
            ensure!(session.state == RUNNING, precondition, "SESSION_NOT_RUNNING", "the timer is not running");
            timer.pause(at);
            Ok(())
        })
        .await
    }

    pub(crate) async fn resume_session(&self, user_id: Uuid) -> AppResult<StudySession> {
        let at = self.now();
        self.change_timer(user_id, at, |session, timer| {
            ensure!(session.state == PAUSED, precondition, "SESSION_NOT_PAUSED", "the timer is not paused");
            timer.run_from(at);
            Ok(())
        })
        .await
    }

    pub(crate) async fn skip_phase(&self, user_id: Uuid) -> AppResult<StudySession> {
        let at = self.now();
        self.change_timer(user_id, at, |session, timer| {
            ensure!(timer.plan != Plan::Open, precondition, "NOT_POMODORO", "a stopwatch has no phases to skip");
            ensure!(session.state == RUNNING, precondition, "SESSION_NOT_RUNNING", "the timer is not running");
            timer.skip(at);
            Ok(())
        })
        .await
    }

    /// Counts the time since the last heartbeat. Returns the session — ended
    /// when the server had to give up on it in the meantime.
    pub(crate) async fn heartbeat(&self, user_id: Uuid) -> AppResult<StudySession> {
        let at = self.now();
        // Clients send one a minute; each is a handful of writes.
        self.core.cache.rate_limit(&format!("school:heartbeat:{user_id}"), 120, StdDuration::from_secs(60)).await?;
        let mut tx = self.core.db.begin().await?;
        match self.active_session(&mut tx, user_id, at).await? {
            Active::None => Err(sessions::no_active_session()),
            Active::Expired(gone) => {
                tx.commit().await?;
                self.announce_gone(&gone, at).await;
                let session = &gone.ended.session;
                Ok(session.to_proto(gone.ended.subject.as_ref(), &session.own_timer(), at, self.idle_limit()))
            }
            Active::Live(mut session, room) => {
                let timer = session.timer(room.as_ref());
                sessions::advance(&mut tx, &mut session, &timer, at).await?;
                sessions::heard_from(&mut tx, &mut session, &timer, at).await?;
                tx.commit().await?;
                self.session_proto(&session, room.as_ref(), at).await
            }
        }
    }

    /// Ends the caller's session (leaving the room it is in) and logs it.
    pub(crate) async fn stop_session(&self, user_id: Uuid, memo: Option<&str>) -> AppResult<StudySession> {
        let at = self.now();
        let memo = memo.map(|memo| text::memo(memo, "memo")).transpose()?;
        let mut tx = self.core.db.begin().await?;
        let gone = match self.active_session(&mut tx, user_id, at).await? {
            Active::None => return Err(sessions::no_active_session()),
            Active::Expired(gone) => gone,
            Active::Live(mut session, room) => {
                if let Some(memo) = &memo {
                    sqlx::query("UPDATE school_study_sessions SET memo = $2 WHERE id = $1")
                        .bind(session.id)
                        .bind(memo)
                        .execute(&mut *tx)
                        .await?;
                    session.memo = memo.clone();
                }
                self.end_locked(&mut tx, session, room, End::Client(at), rooms::Leave::Left, at).await?
            }
        };
        tx.commit().await?;
        self.announce_gone(&gone, at).await;
        let session = &gone.ended.session;
        Ok(session.to_proto(gone.ended.subject.as_ref(), &session.own_timer(), at, self.idle_limit()))
    }

    /// The caller's active session, if any, without touching it.
    pub(crate) async fn current_session(&self, user_id: Uuid) -> AppResult<Option<StudySession>> {
        let at = self.now();
        let session: Option<SessionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM school_study_sessions WHERE user_id = $1 AND state <> {ENDED}",
            sessions::COLUMNS
        )))
        .bind(user_id)
        .fetch_optional(&self.core.db)
        .await?;
        match session.filter(|session| !self.is_overdue(session, at)) {
            Some(session) => Ok(Some(self.session_proto(&session, None, at).await?)),
            None => Ok(None),
        }
    }

    /// Ends the active sessions the limits have caught up with: clients that
    /// fell silent, pauses that never ended, sessions as long as they may
    /// get. Returns how many ended. Safe to run in several processes.
    pub async fn end_overdue_sessions(&self, at: Time) -> AppResult<usize> {
        let idle = at - self.idle_limit();
        let paused = at - chrono::Duration::seconds(self.cfg.session_max_pause_seconds);
        let longest = at - chrono::Duration::seconds(self.cfg.session_max_seconds);
        let overdue: Vec<Uuid> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT user_id FROM school_study_sessions
             WHERE state <> {ENDED}
               AND (started_at <= $3 OR (state = {PAUSED} AND paused_at < $2) OR (state <> {PAUSED} AND believed_until < $1))
             ORDER BY believed_until LIMIT 500"
        )))
        .bind(idle)
        .bind(paused)
        .bind(longest)
        .fetch_all(&self.core.db)
        .await?;
        let mut ended = 0;
        for user_id in overdue {
            let result: AppResult<Option<Gone>> = async {
                let mut tx = self.core.db.begin().await?;
                // Applying the limits is all there is to do; a session that
                // was heard from in the meantime is left alone.
                let gone = match self.active_session(&mut tx, user_id, at).await? {
                    Active::Expired(gone) => Some(gone),
                    _ => None,
                };
                tx.commit().await?;
                Ok(gone)
            }
            .await;
            match result {
                Ok(Some(gone)) => {
                    self.announce_gone(&gone, at).await;
                    ended += 1;
                }
                Ok(None) => {}
                Err(e) => tracing::error!(%user_id, error = %e, "ending an overdue study session failed"),
            }
        }
        Ok(ended)
    }
}
