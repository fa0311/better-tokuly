//! The shared timer of a room, and the reactions that stand in for chat.
use std::time::Duration as StdDuration;

use tokuly_core::prelude::*;
use tokuly_proto::school::{RoomReaction, RoomReactionSent, TimerAction, TimerSnapshot, school_event::Event as Push};

use super::{COLUMNS, RoomRow, lock, not_found, store_timer};
use crate::{
    School,
    study::sessions::{BELIEVED_UNTIL, ENDED},
    timer::{Plan, Run},
};

fn action_invalid(message: &'static str) -> AppError {
    AppError::precondition("TIMER_ACTION_INVALID", message)
}

impl School {
    /// Host: starts, pauses, resumes, skips or resets the room's timer. The
    /// study time of everyone inside is counted up to this moment first, so
    /// the change applies from now on and not to what was already studied.
    pub(crate) async fn control_room_timer(
        &self,
        user_id: Uuid,
        room_id: Uuid,
        action: TimerAction,
    ) -> AppResult<TimerSnapshot> {
        let at = self.now();
        ensure!(
            action != TimerAction::TIMER_ACTION_UNSPECIFIED,
            invalid,
            "TIMER_ACTION_REQUIRED",
            "say what to do with the timer"
        );
        self.hosted_room(room_id, user_id).await?;
        self.core.cache.rate_limit(&format!("school:room-timer:{user_id}"), 60, StdDuration::from_secs(60)).await?;

        let mut tx = self.core.db.begin().await?;
        let mut room = lock(&mut tx, room_id).await?.ok_or_else(not_found)?;
        ensure!(room.is_open(), precondition, "ROOM_CLOSED", "the room is closed");
        let before = room.timer();
        let mut timer = before;
        match (action, timer.run) {
            (TimerAction::TIMER_ACTION_START, Run::Idle) | (TimerAction::TIMER_ACTION_RESUME, Run::Paused) => {
                timer.run_from(at)
            }
            (TimerAction::TIMER_ACTION_START, _) => return Err(action_invalid("the timer has been started already")),
            (TimerAction::TIMER_ACTION_RESUME, _) => return Err(action_invalid("the timer is not paused")),
            (TimerAction::TIMER_ACTION_PAUSE, Run::Running) => timer.pause(at),
            (TimerAction::TIMER_ACTION_PAUSE, _) => return Err(action_invalid("the timer is not running")),
            (TimerAction::TIMER_ACTION_SKIP, Run::Running | Run::Paused) if timer.plan != Plan::Open => timer.skip(at),
            (TimerAction::TIMER_ACTION_SKIP, _) => return Err(action_invalid("there is no phase to skip")),
            (TimerAction::TIMER_ACTION_RESET, _) => timer.reset(),
            (TimerAction::TIMER_ACTION_UNSPECIFIED, _) => unreachable!("rejected above"),
        }
        self.count_members(&mut tx, &room, &before, at).await?;
        store_timer(&mut tx, &mut room, &timer, at).await?;
        tx.commit().await?;

        let snapshot = timer.snapshot(at);
        self.push_room(room.id, Push::RoomTimer(Box::new(snapshot.clone()))).await;
        Ok(snapshot)
    }

    /// Announces the phases that ended by themselves: every room whose
    /// current phase is over gets its timer pushed, so clients that drifted
    /// or just connected line up again. Returns how many rooms moved on.
    /// Safe to run in several processes.
    pub async fn announce_room_phases(&self, at: Time) -> AppResult<usize> {
        let mut tx = self.core.db.begin().await?;
        let mut rooms: Vec<RoomRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM school_rooms r
             WHERE r.closed_at IS NULL AND r.phase_ends_at <= $1
             ORDER BY r.phase_ends_at LIMIT 200 FOR UPDATE SKIP LOCKED"
        )))
        .bind(at)
        .fetch_all(&mut *tx)
        .await?;
        for room in &mut rooms {
            // Nothing about the timer changes: it has moved on by itself.
            // Only the next moment to announce does.
            sqlx::query("UPDATE school_rooms SET phase_ends_at = $2 WHERE id = $1")
                .bind(room.id)
                .bind(room.timer().phase_ends_at(at))
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        for room in &rooms {
            self.push_room(room.id, Push::RoomTimer(Box::new(room.timer().snapshot(at)))).await;
        }
        Ok(rooms.len())
    }

    /// Sends one of the fixed reactions to the caller's room, or to one of
    /// the people in it.
    pub(crate) async fn send_room_reaction(
        &self,
        user_id: Uuid,
        reaction: RoomReaction,
        to: Option<Uuid>,
    ) -> AppResult<()> {
        let core = &self.core;
        let at = self.now();
        ensure!(reaction != RoomReaction::ROOM_REACTION_UNSPECIFIED, invalid, "REACTION_REQUIRED", "choose a reaction");
        let room_id: Option<Uuid> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT room_id FROM school_study_sessions WHERE user_id = $1 AND state <> {ENDED} AND room_id IS NOT NULL AND {believed} >= $2",
            believed = BELIEVED_UNTIL
        )))
        .bind(user_id)
        .bind(at - self.idle_limit())
        .fetch_optional(&core.db)
        .await?;
        let room_id = room_id.ok_or_else(|| AppError::precondition("NOT_IN_ROOM", "you are not in a study room"))?;
        if let Some(to) = to {
            let present: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT EXISTS(SELECT 1 FROM school_study_sessions WHERE user_id = $1 AND room_id = $2 AND state <> {ENDED})"
            )))
            .bind(to)
            .bind(room_id)
            .fetch_one(&core.db)
            .await?;
            ensure!(present && to != user_id, not_found, "USER_NOT_IN_ROOM", "this person is not in the room");
        }
        core.cache.rate_limit(&format!("school:room-reaction:{user_id}"), 30, StdDuration::from_secs(60)).await?;
        let event = RoomReactionSent {
            from: tokuly_identity::summary(core, user_id).await?.into(),
            reaction: reaction.into(),
            to_user_id: to.map(|id| id.to_string()).unwrap_or_default(),
            ..Default::default()
        };
        self.push_room(room_id, Push::RoomReaction(Box::new(event))).await;
        Ok(())
    }
}
