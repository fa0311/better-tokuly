//! Study rooms (もくもく部屋).
//!
//! A room is a shared timer plus the people following it. Its members are
//! the active study sessions that point at it, so "who is in the room" and
//! "whose study time is counting" are one fact, and leaving a room is
//! ending a session.
//!
//! Locking: every operation on a room takes the room's row lock first and
//! the sessions of its members after that, and never holds two rooms.
//!
//! - this file: the room row, who may see and enter a room, the API message;
//! - `lifecycle`: creating, changing and closing rooms, joining, leaving,
//!   invitations and removals;
//! - `control`: the shared timer, phase announcements and reactions;
//! - `svc`: the RPC handlers.
use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use tokuly_core::prelude::*;
use tokuly_proto::{
    common::UserSummary,
    school::{
        RoomAudience, RoomLeaveReason, RoomLeft, RoomMember, RoomMemberLeft, StudyRoom, StudySession,
        school_event::Event as Push,
    },
};
use tokuly_social::graph;

use crate::{
    School,
    study::sessions::{self, ENDED, SessionRow},
    subjects,
    timer::{Plan, Run, Timer},
};

mod control;
mod lifecycle;
mod svc;

pub(crate) const MAX_MEMBER_CAP: i32 = 50;
/// Members shown with a room in lists.
const MEMBER_PREVIEW: usize = 5;

pub(crate) const AUDIENCE_PUBLIC: i16 = RoomAudience::ROOM_AUDIENCE_PUBLIC as i16;
pub(crate) const AUDIENCE_FRIENDS: i16 = RoomAudience::ROOM_AUDIENCE_FRIENDS as i16;

/// The realtime topic of a room.
pub(crate) fn topic(room_id: Uuid) -> String {
    format!("study:{room_id}")
}

/// Path of a room in the web app.
pub(crate) fn path(room_id: Uuid) -> String {
    format!("/school/rooms/{room_id}")
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct RoomRow {
    pub id: Uuid,
    pub host_id: Uuid,
    pub title: String,
    pub audience: i16,
    pub member_cap: i32,
    pub mode: i16,
    pub work_seconds: i32,
    pub break_seconds: i32,
    pub long_break_seconds: i32,
    pub rounds: i32,
    pub timer_state: i16,
    pub timer_anchor_at: Option<DateTime<Utc>>,
    pub timer_offset_ms: i64,
    pub member_count: i32,
    pub empty_since: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

pub(crate) const COLUMNS: &str = "r.id, r.host_id, r.title, r.audience, r.member_cap, r.mode, r.work_seconds, r.break_seconds, r.long_break_seconds, \
     r.rounds, r.timer_state, r.timer_anchor_at, r.timer_offset_ms, r.member_count, r.empty_since, r.closed_at, r.created_at";

impl RoomRow {
    /// The shared timer every member counts by.
    pub fn timer(&self) -> Timer {
        Timer {
            plan: Plan::from_columns(
                self.mode,
                self.work_seconds,
                self.break_seconds,
                self.long_break_seconds,
                self.rounds,
            ),
            run: Run::from_column(self.timer_state),
            anchor: self.timer_anchor_at,
            offset_ms: self.timer_offset_ms,
        }
    }

    pub fn is_open(&self) -> bool {
        self.closed_at.is_none()
    }

    pub fn is_public(&self) -> bool {
        self.audience == AUDIENCE_PUBLIC
    }
}

/// Why someone is no longer in a room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Leave {
    Left,
    Idle,
    Removed,
}

impl Leave {
    fn reason(self) -> RoomLeaveReason {
        match self {
            Self::Left => RoomLeaveReason::ROOM_LEAVE_REASON_LEFT,
            Self::Idle => RoomLeaveReason::ROOM_LEAVE_REASON_IDLE,
            Self::Removed => RoomLeaveReason::ROOM_LEAVE_REASON_REMOVED,
        }
    }
}

/// What a member's departure did to the room, to announce after the commit.
#[derive(Debug, Clone)]
pub(crate) struct RoomExit {
    pub room_id: Uuid,
    pub member_count: i32,
    pub leave: Leave,
    /// Set when the departure changed the shared timer (paused because the
    /// room is empty, started because the host left).
    pub timer: Option<Timer>,
}

fn not_found() -> AppError {
    AppError::not_found("ROOM_NOT_FOUND", "study room not found")
}

/// Locks a room for the rest of the transaction.
pub(crate) async fn lock(conn: &mut PgConnection, room_id: Uuid) -> AppResult<Option<RoomRow>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM school_rooms r WHERE r.id = $1 FOR UPDATE")))
        .bind(room_id)
        .fetch_optional(conn)
        .await?)
}

pub(crate) async fn find(core: &Core, room_id: Uuid) -> AppResult<Option<RoomRow>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM school_rooms r WHERE r.id = $1")))
        .bind(room_id)
        .fetch_optional(&core.db)
        .await?)
}

pub(crate) async fn by_ids(core: &Core, ids: &[Uuid]) -> AppResult<HashMap<Uuid, RoomRow>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<RoomRow> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM school_rooms r WHERE r.id = ANY($1)")))
            .bind(ids)
            .fetch_all(&core.db)
            .await?;
    Ok(rows.into_iter().map(|row| (row.id, row)).collect())
}

/// Writes the room's timer back.
async fn store_timer(conn: &mut PgConnection, room: &mut RoomRow, timer: &Timer, at: Time) -> AppResult<()> {
    let (work, short_break, long_break, rounds) = timer.plan.columns();
    room.mode = timer.plan.mode() as i16;
    (room.work_seconds, room.break_seconds, room.long_break_seconds, room.rounds) =
        (work, short_break, long_break, rounds);
    room.timer_state = timer.run.column();
    room.timer_anchor_at = timer.anchor;
    room.timer_offset_ms = timer.offset_ms;
    sqlx::query(
        "UPDATE school_rooms SET mode = $2, work_seconds = $3, break_seconds = $4, long_break_seconds = $5, rounds = $6,
             timer_state = $7, timer_anchor_at = $8, timer_offset_ms = $9, phase_ends_at = $10
         WHERE id = $1",
    )
    .bind(room.id)
    .bind(room.mode)
    .bind(work)
    .bind(short_break)
    .bind(long_break)
    .bind(rounds)
    .bind(room.timer_state)
    .bind(room.timer_anchor_at)
    .bind(room.timer_offset_ms)
    .bind(timer.phase_ends_at(at))
    .execute(conn)
    .await?;
    Ok(())
}

/// The sessions of the people in a room, in the order they came in. With
/// `for_update`, locked (the caller holds the room's lock).
async fn member_sessions(conn: &mut PgConnection, room_id: Uuid, for_update: bool) -> AppResult<Vec<SessionRow>> {
    let lock = if for_update { " FOR UPDATE" } else { "" };
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {} FROM school_study_sessions WHERE room_id = $1 AND state <> {ENDED} ORDER BY room_joined_at, id{lock}",
        sessions::COLUMNS
    )))
    .bind(room_id)
    .fetch_all(conn)
    .await?)
}

impl School {
    /// Whether an open room is one `viewer` may enter.
    pub(crate) async fn can_enter(&self, room: &RoomRow, viewer: Uuid) -> AppResult<bool> {
        Ok(room.is_open() && self.may_see(room, viewer).await?)
    }

    /// Whether the room exists for `viewer`: their own, a public one, a
    /// friend's friends-only one, one they are invited to or already in —
    /// unless the host removed them, or they are in a block with the host
    /// or with someone inside.
    pub(crate) async fn may_see(&self, room: &RoomRow, viewer: Uuid) -> AppResult<bool> {
        let core = &self.core;
        if viewer == room.host_id {
            return Ok(true);
        }
        let (banned, invited, inside): (bool, bool, bool) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT EXISTS(SELECT 1 FROM school_room_bans WHERE room_id = $1 AND user_id = $2),
                    EXISTS(SELECT 1 FROM school_room_invites WHERE room_id = $1 AND user_id = $2 AND withdrawn_at IS NULL),
                    EXISTS(SELECT 1 FROM school_study_sessions WHERE room_id = $1 AND user_id = $2 AND state <> {ENDED})"
        )))
        .bind(room.id)
        .bind(viewer)
        .fetch_one(&core.db)
        .await?;
        if banned || graph::is_blocked_either(core, room.host_id, viewer).await? {
            return Ok(false);
        }
        // Someone inside keeps seeing the room they are in.
        if inside {
            return Ok(true);
        }
        let allowed = room.is_public()
            || invited
            || (room.audience == AUDIENCE_FRIENDS && graph::are_friends(core, room.host_id, viewer).await?);
        if !allowed {
            return Ok(false);
        }
        // Blocks hide people from each other everywhere, rooms included.
        let hidden = graph::blocked_ids_either(core, viewer).await?;
        if hidden.is_empty() {
            return Ok(true);
        }
        let blocked_inside: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT EXISTS(SELECT 1 FROM school_study_sessions WHERE room_id = $1 AND state <> {ENDED} AND user_id = ANY($2))"
        )))
        .bind(room.id)
        .bind(&hidden)
        .fetch_one(&core.db)
        .await?;
        Ok(!blocked_inside)
    }

    /// Whether the room was put in front of `viewer` at some point, whatever
    /// became of it: they were invited (even if the invitation was withdrawn
    /// since), they were inside, or the host removed them. Someone who was
    /// shown a room can report it, also after it is closed to them.
    pub(crate) async fn was_shown(&self, room: &RoomRow, viewer: Uuid) -> AppResult<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM school_room_invites WHERE room_id = $1 AND user_id = $2)
                 OR EXISTS(SELECT 1 FROM school_room_bans WHERE room_id = $1 AND user_id = $2)
                 OR EXISTS(SELECT 1 FROM school_study_sessions WHERE room_id = $1 AND user_id = $2)",
        )
        .bind(room.id)
        .bind(viewer)
        .fetch_one(&self.core.db)
        .await?)
    }

    /// The room `viewer` asked for, if it exists for them (closed or not).
    pub(crate) async fn visible_room(&self, room_id: Uuid, viewer: Uuid) -> AppResult<RoomRow> {
        let room = find(&self.core, room_id).await?.ok_or_else(not_found)?;
        // A closed room is gone for everyone but its host.
        let visible = if room.is_open() { self.may_see(&room, viewer).await? } else { viewer == room.host_id };
        if visible { Ok(room) } else { Err(not_found()) }
    }

    /// One of the caller's own rooms, for host-only operations.
    async fn hosted_room(&self, room_id: Uuid, viewer: Uuid) -> AppResult<RoomRow> {
        let room = self.visible_room(room_id, viewer).await?;
        ensure!(room.host_id == viewer, denied, "NOT_HOST", "only the host can do this");
        Ok(room)
    }

    /// Rooms as `viewer` sees them. `preview` limits the members listed per
    /// room; `None` lists everyone.
    pub(crate) async fn room_protos(
        &self,
        rooms: &[RoomRow],
        viewer: Uuid,
        at: Time,
        preview: Option<usize>,
    ) -> AppResult<Vec<StudyRoom>> {
        let core = &self.core;
        if rooms.is_empty() {
            return Ok(Vec::new());
        }
        let room_ids: Vec<Uuid> = rooms.iter().map(|room| room.id).collect();
        let members: Vec<SessionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM school_study_sessions WHERE room_id = ANY($1) AND state <> {ENDED} ORDER BY room_joined_at, id",
            sessions::COLUMNS
        )))
        .bind(&room_ids)
        .fetch_all(&core.db)
        .await?;
        let invites: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT room_id, user_id FROM school_room_invites
             WHERE room_id = ANY($1) AND withdrawn_at IS NULL ORDER BY created_at, user_id",
        )
        .bind(&room_ids)
        .fetch_all(&core.db)
        .await?;
        let hidden: HashSet<Uuid> = graph::blocked_ids_either(core, viewer).await?.into_iter().collect();

        let mut user_ids: Vec<Uuid> = rooms.iter().map(|room| room.host_id).collect();
        user_ids.extend(members.iter().map(|member| member.user_id));
        user_ids.extend(invites.iter().map(|(_, user_id)| *user_id));
        let users = tokuly_identity::summaries(core, &user_ids).await?;
        let subjects =
            subjects::by_ids(core, &members.iter().filter_map(|member| member.subject_id).collect::<Vec<_>>()).await?;

        Ok(rooms
            .iter()
            .map(|room| {
                let timer = room.timer();
                let inside: Vec<&SessionRow> =
                    members.iter().filter(|member| member.room_id == Some(room.id)).collect();
                let is_host = viewer == room.host_id;
                let invited: Vec<Uuid> =
                    invites.iter().filter(|(room_id, _)| *room_id == room.id).map(|(_, user_id)| *user_id).collect();
                let known = |id: &Uuid| -> Option<UserSummary> { users.get(id).filter(|user| !user.deleted).cloned() };
                StudyRoom {
                    id: room.id.to_string(),
                    title: room.title.clone(),
                    host: tokuly_identity::users::pick(&users, room.host_id).into(),
                    audience: (room.audience as i32).into(),
                    timer: timer.snapshot(at).into(),
                    // People in a block with the viewer are not listed, and
                    // not counted either: a count one higher than the list
                    // would say that someone hidden is there.
                    member_count: room.member_count
                        - inside.iter().filter(|member| hidden.contains(&member.user_id)).count() as i32,
                    member_cap: room.member_cap,
                    members: inside
                        .iter()
                        .filter(|member| !hidden.contains(&member.user_id))
                        .take(preview.unwrap_or(usize::MAX))
                        .filter_map(|member| {
                            Some(self.member_proto(
                                member,
                                room,
                                known(&member.user_id)?,
                                member.subject_id.and_then(|id| subjects.get(&id)),
                                at,
                            ))
                        })
                        .collect(),
                    joined: inside.iter().any(|member| member.user_id == viewer),
                    host_present: inside.iter().any(|member| member.user_id == room.host_id),
                    invited: invited.contains(&viewer),
                    invitees: if is_host { invited.iter().filter_map(known).collect() } else { Vec::new() },
                    topic: topic(room.id),
                    created_at: ts(room.created_at),
                    closed_at: opt_ts(room.closed_at),
                    ..Default::default()
                }
            })
            .collect())
    }

    fn member_proto(
        &self,
        member: &SessionRow,
        room: &RoomRow,
        user: UserSummary,
        subject: Option<&subjects::SubjectRow>,
        at: Time,
    ) -> RoomMember {
        RoomMember {
            user: user.into(),
            subject: subject.map(|subject| subject.label()).into(),
            joined_at: ts(member.room_joined_at.unwrap_or(member.started_at)),
            studied_seconds: member.studied_seconds(&room.timer(), at, self.idle_limit()),
            host: member.user_id == room.host_id,
            ..Default::default()
        }
    }

    /// One room with everyone in it.
    pub(crate) async fn room_proto(&self, room: &RoomRow, viewer: Uuid, at: Time) -> AppResult<StudyRoom> {
        Ok(self
            .room_protos(std::slice::from_ref(room), viewer, at, None)
            .await?
            .pop()
            .expect("one room in, one room out"))
    }

    /// A member as the room's topic announces them.
    pub(crate) async fn member_event(&self, session: &SessionRow, room: &RoomRow, at: Time) -> AppResult<RoomMember> {
        let core = &self.core;
        let user = tokuly_identity::summary(core, session.user_id).await?;
        let subject = match session.subject_id {
            Some(id) => subjects::by_ids(core, &[id]).await?.remove(&id),
            None => None,
        };
        Ok(self.member_proto(session, room, user, subject.as_ref(), at))
    }

    /// Announces that someone is out of a room: to the room, and — unless
    /// they left by themselves — to their own devices.
    pub(crate) async fn announce_exit(&self, user_id: Uuid, exit: &RoomExit, session: &StudySession, at: Time) {
        let left = RoomMemberLeft {
            user_id: user_id.to_string(),
            reason: exit.leave.reason().into(),
            member_count: exit.member_count,
            ..Default::default()
        };
        self.push_room(exit.room_id, Push::RoomMemberLeft(Box::new(left))).await;
        if let Some(timer) = &exit.timer {
            self.push_room(exit.room_id, Push::RoomTimer(Box::new(timer.snapshot(at)))).await;
        }
        if exit.leave != Leave::Left {
            let event = RoomLeft {
                room_id: exit.room_id.to_string(),
                reason: exit.leave.reason().into(),
                session: session.clone().into(),
                ..Default::default()
            };
            self.push(user_id, Push::RoomLeft(Box::new(event))).await;
        }
    }
}
