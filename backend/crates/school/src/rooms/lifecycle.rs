//! The life of a study room: opening, changing and closing it, coming and
//! going, invitations and removals.
use std::{collections::HashSet, time::Duration as StdDuration};

use chrono::Duration;
use sqlx::{Postgres, Transaction};
use tokuly_core::{
    events,
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::{
    common::{PageInfo, SubjectType, UserSummary},
    school::{
        CreateStudyRoomRequest, RoomAudience, RoomCloseReason, RoomClosed as RoomClosedEvent, RoomLeaveReason,
        RoomLeft, RoomScope, StudyMode, StudyRoom, StudySession, UpdateStudyRoomRequest, school_event::Event as Push,
    },
};
use tokuly_social::graph;

use super::{
    AUDIENCE_FRIENDS, AUDIENCE_PUBLIC, COLUMNS, Leave, MAX_MEMBER_CAP, MEMBER_PREVIEW, RoomExit, RoomRow, lock,
    member_sessions, not_found, path, store_timer, topic,
};
use crate::{
    School,
    events::{ROOM_CLOSED, ROOM_OPENED, RoomClosed, RoomOpened},
    settings,
    study::{
        focus::{Active, Gone},
        sessions::{self, ENDED, End, Ended, NewSession, SOURCE_ROOM, SessionRow},
    },
    subjects, text,
    timer::{Plan, Timer},
};

pub(crate) const KIND_ROOM_INVITE: &str = "school.room_invite";
const MAX_INVITES_PER_CALL: usize = 50;
const MAX_INVITES: i64 = 200;
/// Invitation notifications one host may cause one person per hour.
const INVITE_NOTIFICATIONS_PER_HOUR: u32 = 3;

fn check_audience(audience: RoomAudience) -> AppResult<i16> {
    match audience {
        RoomAudience::ROOM_AUDIENCE_PUBLIC
        | RoomAudience::ROOM_AUDIENCE_FRIENDS
        | RoomAudience::ROOM_AUDIENCE_INVITED => Ok(audience as i16),
        RoomAudience::ROOM_AUDIENCE_UNSPECIFIED => Err(AppError::invalid("AUDIENCE_INVALID", "unknown audience")),
    }
}

fn check_cap(cap: i32) -> AppResult<i32> {
    ensure!(
        (2..=MAX_MEMBER_CAP).contains(&cap),
        invalid,
        "MEMBER_CAP_INVALID",
        "a room holds 2 to {MAX_MEMBER_CAP} people"
    );
    Ok(cap)
}

fn ensure_open(room: &RoomRow) -> AppResult<()> {
    ensure!(room.is_open(), precondition, "ROOM_CLOSED", "the room is closed");
    Ok(())
}

impl School {
    /// Takes a member out of a room; the caller holds the locks on the room
    /// and the session. The session ends (and is logged), and the room keeps
    /// going for the others: an empty room pauses its timer and starts
    /// waiting to be closed; a room whose host left with the timer stopped
    /// starts it, so nobody is left waiting.
    pub(crate) async fn depart(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        room: &mut RoomRow,
        session: SessionRow,
        end: End,
        leave: Leave,
        at: Time,
    ) -> AppResult<(Ended, RoomExit)> {
        let user_id = session.user_id;
        let before = room.timer();
        let ended = self.finish(tx, session, &before, end).await?;
        let remaining: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM school_study_sessions WHERE room_id = $1 AND state <> {ENDED}"
        )))
        .bind(room.id)
        .fetch_one(&mut **tx)
        .await?;
        room.member_count = remaining as i32;
        let mut timer = before;
        if remaining == 0 {
            timer.pause(at);
            room.empty_since = Some(at);
        } else if user_id == room.host_id {
            timer.run_from(at);
        }
        sqlx::query("UPDATE school_rooms SET member_count = $2, empty_since = $3 WHERE id = $1")
            .bind(room.id)
            .bind(room.member_count)
            .bind(room.empty_since)
            .execute(&mut **tx)
            .await?;
        let changed = timer != before;
        if changed {
            store_timer(tx, room, &timer, at).await?;
        }
        Ok((
            ended,
            RoomExit { room_id: room.id, member_count: room.member_count, leave, timer: changed.then_some(timer) },
        ))
    }

    /// Ends whatever session the user has going — it is logged — in a
    /// transaction of its own, before they enter `entering`. (Entering never
    /// holds the locks of two rooms.) A stay in that very room is left alone
    /// and returned: the user is inside already.
    async fn end_current_session(&self, user_id: Uuid, entering: Uuid, at: Time) -> AppResult<Option<SessionRow>> {
        let mut tx = self.core.db.begin().await?;
        let gone = match self.active_session(&mut tx, user_id, at).await? {
            Active::None => None,
            Active::Expired(gone) => Some(gone),
            Active::Live(session, _) if session.room_id == Some(entering) => return Ok(Some(session)),
            Active::Live(session, room) => {
                Some(self.end_locked(&mut tx, session, room, End::Client(at), Leave::Left, at).await?)
            }
        };
        tx.commit().await?;
        if let Some(gone) = &gone {
            self.announce_gone(gone, at).await;
        }
        Ok(None)
    }

    /// Closes a room: everyone inside is out, their sessions end and are
    /// logged. `closer` is the member who closed it themselves (their time
    /// counts up to now; everyone else's up to their last heartbeat). Returns
    /// false when the room was closed already (or never existed).
    pub(crate) async fn close_room(
        &self,
        room_id: Uuid,
        reason: RoomCloseReason,
        closer: Option<Uuid>,
        at: Time,
    ) -> AppResult<bool> {
        let mut tx = self.core.db.begin().await?;
        let Some(room) = lock(&mut tx, room_id).await?.filter(RoomRow::is_open) else { return Ok(false) };
        let ended = self.close_locked(&mut tx, &room, reason, closer, at).await?;
        tx.commit().await?;
        self.announce_closed(&room, reason, &ended, at).await;
        // After `room_closed`, which travels on the same topic and so arrives first.
        self.core.rt.close_topic(&topic(room.id)).await;
        Ok(true)
    }

    async fn close_locked(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        room: &RoomRow,
        reason: RoomCloseReason,
        closer: Option<Uuid>,
        at: Time,
    ) -> AppResult<Vec<Ended>> {
        let timer = room.timer();
        let mut ended = Vec::new();
        for session in member_sessions(tx, room.id, true).await? {
            let end = if Some(session.user_id) == closer { End::Client(at) } else { End::Server(at) };
            ended.push(self.finish(tx, session, &timer, end).await?);
        }
        sqlx::query(
            "UPDATE school_rooms SET closed_at = $2, close_reason = $3, member_count = 0, empty_since = NULL, phase_ends_at = NULL,
                 timer_state = 1, timer_anchor_at = NULL, timer_offset_ms = 0
             WHERE id = $1",
        )
        .bind(room.id)
        .bind(at)
        .bind(reason as i16)
        .execute(&mut **tx)
        .await?;
        let event = RoomClosed { room_id: room.id, host_id: room.host_id, reason: reason as i32 };
        events::emit(&mut **tx, ROOM_CLOSED, Some(room.host_id), &event).await?;
        Ok(ended)
    }

    async fn announce_closed(&self, room: &RoomRow, reason: RoomCloseReason, ended: &[Ended], at: Time) {
        let event = RoomClosedEvent { room_id: room.id.to_string(), reason: reason.into(), ..Default::default() };
        self.push_room(room.id, Push::RoomClosed(Box::new(event))).await;
        for gone in ended {
            let session = &gone.session;
            let proto = session.to_proto(gone.subject.as_ref(), &session.own_timer(), at, self.idle_limit());
            let left = RoomLeft {
                room_id: room.id.to_string(),
                reason: RoomLeaveReason::ROOM_LEAVE_REASON_CLOSED.into(),
                session: proto.clone().into(),
                ..Default::default()
            };
            self.push(session.user_id, Push::RoomLeft(Box::new(left))).await;
            self.push(session.user_id, Push::Session(Box::new(proto))).await;
            self.announce_studying_quietly(session.user_id, at).await;
            self.track_logged(session).await;
        }
    }

    /// Closes the rooms nobody has been in for a while. Returns how many.
    pub async fn close_empty_rooms(&self, at: Time) -> AppResult<usize> {
        let cutoff = at - Duration::seconds(self.cfg.room_empty_close_seconds);
        let empty: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM school_rooms WHERE closed_at IS NULL AND empty_since <= $1 ORDER BY empty_since LIMIT 200",
        )
        .bind(cutoff)
        .fetch_all(&self.core.db)
        .await?;
        let mut closed = 0;
        for room_id in empty {
            let result: AppResult<bool> = async {
                let mut tx = self.core.db.begin().await?;
                // Someone may have come back in the meantime.
                let Some(room) = lock(&mut tx, room_id)
                    .await?
                    .filter(|room| room.is_open() && room.empty_since.is_some_and(|since| since <= cutoff))
                else {
                    return Ok(false);
                };
                let ended =
                    self.close_locked(&mut tx, &room, RoomCloseReason::ROOM_CLOSE_REASON_EMPTY, None, at).await?;
                tx.commit().await?;
                self.announce_closed(&room, RoomCloseReason::ROOM_CLOSE_REASON_EMPTY, &ended, at).await;
                Ok(true)
            }
            .await;
            match result {
                Ok(true) => closed += 1,
                Ok(false) => {}
                Err(e) => tracing::error!(%room_id, error = %e, "closing an empty study room failed"),
            }
        }
        Ok(closed)
    }

    /// The friends of the host among `ids` who may be invited.
    async fn check_invitees(&self, host: Uuid, ids: &[String]) -> AppResult<Vec<Uuid>> {
        ensure!(
            ids.len() <= MAX_INVITES_PER_CALL,
            invalid,
            "TOO_MANY_USERS",
            "invite at most {MAX_INVITES_PER_CALL} people at a time"
        );
        let mut seen = HashSet::new();
        let ids: Vec<Uuid> = parse_ids(ids)?.into_iter().filter(|id| *id != host && seen.insert(*id)).collect();
        let friends: HashSet<Uuid> = graph::friend_ids(&self.core, host).await?.into_iter().collect();
        ensure!(ids.iter().all(|id| friends.contains(id)), precondition, "NOT_FRIENDS", "only friends can be invited");
        Ok(ids)
    }

    /// Stores invitations and requests their notifications; returns the
    /// people who were not invited before (or whose invitation had been
    /// withdrawn). The caller holds the room's lock.
    async fn add_invites(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        room: &RoomRow,
        host: &UserSummary,
        ids: &[Uuid],
    ) -> AppResult<Vec<Uuid>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let new: Vec<Uuid> = sqlx::query_scalar(
            "INSERT INTO school_room_invites (room_id, user_id) SELECT $1, unnest($2::uuid[])
             ON CONFLICT (room_id, user_id) DO UPDATE SET withdrawn_at = NULL, created_at = now()
                 WHERE school_room_invites.withdrawn_at IS NOT NULL
             RETURNING user_id",
        )
        .bind(room.id)
        .bind(ids)
        .fetch_all(&mut **tx)
        .await?;
        let total: i64 =
            sqlx::query_scalar("SELECT count(*) FROM school_room_invites WHERE room_id = $1 AND withdrawn_at IS NULL")
                .bind(room.id)
                .fetch_one(&mut **tx)
                .await?;
        ensure!(total <= MAX_INVITES, invalid, "TOO_MANY_USERS", "a room has at most {MAX_INVITES} invitations");
        for invitee in &new {
            // Inviting, withdrawing and inviting again must not become a way
            // to ring someone's phone: a few notifications per friend an hour.
            let key = format!("school:room-invite:{}:{invitee}", room.host_id);
            if self
                .core
                .cache
                .rate_limit(&key, INVITE_NOTIFICATIONS_PER_HOUR, StdDuration::from_secs(3600))
                .await
                .is_err()
            {
                continue;
            }
            let request = NotificationRequest::new(
                *invitee,
                KIND_ROOM_INVITE,
                Localized::new("もくもく部屋に招待されました!", "You're invited to a study room"),
                Localized::new(
                    format!("{}さんの「{}」で一緒に勉強しよう📚", host.display_name, room.title),
                    format!("Study with {} in \"{}\" 📚", host.display_name, room.title),
                ),
                path(room.id),
            )
            .actor(room.host_id)
            .subject(SubjectType::SUBJECT_TYPE_STUDY_ROOM, room.id)
            .collapse(format!("{KIND_ROOM_INVITE}:{}", room.id));
            outbound::notify(&mut **tx, &request).await?;
        }
        Ok(new)
    }

    /// Shows each new invitee the room on their devices.
    async fn announce_invites(&self, room: &RoomRow, invitees: &[Uuid], at: Time) {
        for invitee in invitees {
            match self.room_protos(std::slice::from_ref(room), *invitee, at, Some(MEMBER_PREVIEW)).await {
                Ok(mut rooms) => {
                    if let Some(room) = rooms.pop() {
                        self.push(*invitee, Push::RoomInvitation(Box::new(room))).await;
                    }
                }
                Err(e) => tracing::warn!(room_id = %room.id, error = %e, "announcing a study room invitation failed"),
            }
        }
    }

    /// Opens a room with the caller inside as its host.
    pub(crate) async fn create_room(
        &self,
        user_id: Uuid,
        request: &CreateStudyRoomRequest,
    ) -> AppResult<(RoomRow, SessionRow)> {
        let core = &self.core;
        let at = self.now();
        let title = text::name(&request.title, text::MAX_NAME, "ROOM_TITLE_INVALID", "room title")?;
        let audience = match request.audience.as_known() {
            Some(RoomAudience::ROOM_AUDIENCE_UNSPECIFIED) => AUDIENCE_FRIENDS,
            Some(audience) => check_audience(audience)?,
            None => return Err(AppError::invalid("AUDIENCE_INVALID", "unknown audience")),
        };
        let mode = match request.mode.as_known() {
            Some(StudyMode::STUDY_MODE_UNSPECIFIED) => StudyMode::STUDY_MODE_POMODORO,
            Some(mode) => mode,
            None => return Err(AppError::invalid("STUDY_MODE_REQUIRED", "unknown timer mode")),
        };
        let plan = Self::plan(mode, request.pomodoro.as_option(), settings::of(core, user_id).await?.pomodoro())?;
        let member_cap =
            if request.member_cap == 0 { self.cfg.room_default_member_cap } else { check_cap(request.member_cap)? };
        let subject = subjects::require_opt(core, user_id, &request.subject_id).await?;
        let invitees = self.check_invitees(user_id, &request.invite_user_ids).await?;
        core.cache.rate_limit(&format!("school:room-create:{user_id}"), 10, StdDuration::from_secs(3600)).await?;

        let host = tokuly_identity::summary(core, user_id).await?;

        let mut tx = core.db.begin().await?;
        crate::lock_user(&mut tx, "session", user_id).await?;
        // One open room per host. A room they walked out of and nobody is in
        // makes way; one with people inside does not. (Decided under the
        // user's lock: two requests at once open one room, not two.)
        let hosting: Option<RoomRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM school_rooms r WHERE r.host_id = $1 AND r.closed_at IS NULL FOR UPDATE"
        )))
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;
        let mut made_way = None;
        if let Some(old) = hosting {
            let inside = member_sessions(&mut tx, old.id, false).await?;
            ensure!(inside.is_empty(), precondition, "ALREADY_HOSTING", "you already host a study room");
            let ended = self.close_locked(&mut tx, &old, RoomCloseReason::ROOM_CLOSE_REASON_EMPTY, None, at).await?;
            made_way = Some((old, ended));
        }
        // Whatever the host had going ends (and is logged) with the same
        // transaction; their own room above was empty, so this is another
        // room at most, and the new room does not exist for anyone yet.
        let gone = match self.active_session(&mut tx, user_id, at).await? {
            Active::None => None,
            Active::Expired(gone) => Some(gone),
            Active::Live(session, room) => {
                Some(self.end_locked(&mut tx, session, room, End::Client(at), Leave::Left, at).await?)
            }
        };
        let (work, short_break, long_break, rounds) = plan.columns();
        let room: RoomRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO school_rooms AS r (host_id, title, audience, member_cap, mode, work_seconds, break_seconds, long_break_seconds, rounds, member_count)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 1) RETURNING {COLUMNS}"
        )))
        .bind(user_id)
        .bind(&title)
        .bind(audience)
        .bind(member_cap)
        .bind(plan.mode() as i16)
        .bind(work)
        .bind(short_break)
        .bind(long_break)
        .bind(rounds)
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| match &error {
            sqlx::Error::Database(db) if db.is_unique_violation() => AppError::precondition("ALREADY_HOSTING", "you already host a study room"),
            _ => error.into(),
        })?;
        let new = NewSession {
            user_id,
            subject_id: subject.as_ref().map(|s| s.id),
            memo: "",
            source: SOURCE_ROOM,
            plan: Plan::Open,
            room: Some((room.id, at)),
            at,
        };
        let session = sessions::insert(&mut tx, &new).await?;
        let invited = self.add_invites(&mut tx, &room, &host, &invitees).await?;
        let opened = RoomOpened {
            room_id: room.id,
            host_id: user_id,
            audience: room.audience as i32,
            title: room.title.clone(),
        };
        events::emit(&mut *tx, ROOM_OPENED, Some(user_id), &opened).await?;
        tx.commit().await?;

        if let Some((old, ended)) = &made_way {
            self.announce_closed(old, RoomCloseReason::ROOM_CLOSE_REASON_EMPTY, ended, at).await;
        }
        if let Some(gone) = &gone {
            self.announce_gone(gone, at).await;
        }
        self.announce_invites(&room, &invited, at).await;
        self.announce_entered(&room, &session, at).await;
        Ok((room, session))
    }

    /// Tells the newcomer's devices, the room and their friends that they are in.
    async fn announce_entered(&self, room: &RoomRow, session: &SessionRow, at: Time) {
        // Blocks hide people from each other in rooms too: someone who follows
        // the room from outside and is in a block with the newcomer may no
        // longer follow it. Only worth asking everyone when there is a block.
        match graph::blocked_ids_either(&self.core, session.user_id).await {
            Ok(hidden) if hidden.is_empty() => {}
            Ok(_) => self.core.rt.recheck_topic(&topic(room.id)).await,
            Err(e) => {
                tracing::warn!(error = %e, "blocks of a newcomer unknown; asking the room's followers again");
                self.core.rt.recheck_topic(&topic(room.id)).await;
            }
        }
        match self.session_proto(session, Some(room), at).await {
            Ok(proto) => self.push(session.user_id, Push::Session(Box::new(proto))).await,
            Err(e) => tracing::warn!(error = %e, "announcing a study session failed"),
        }
        match self.member_event(session, room, at).await {
            Ok(member) => self.push_room(room.id, Push::RoomMemberJoined(Box::new(member))).await,
            Err(e) => tracing::warn!(error = %e, "announcing a study room member failed"),
        }
        self.announce_studying_quietly(session.user_id, at).await;
    }

    pub(crate) async fn update_room(&self, user_id: Uuid, request: &UpdateStudyRoomRequest) -> AppResult<RoomRow> {
        let core = &self.core;
        let at = self.now();
        let room_id = parse_id(&request.room_id)?;
        let current = self.hosted_room(room_id, user_id).await?;
        ensure_open(&current)?;
        let title = request
            .title
            .as_deref()
            .map(|title| text::name(title, text::MAX_NAME, "ROOM_TITLE_INVALID", "room title"))
            .transpose()?;
        let audience = match request.audience.as_known() {
            Some(RoomAudience::ROOM_AUDIENCE_UNSPECIFIED) => None,
            Some(audience) => Some(check_audience(audience)?),
            None => return Err(AppError::invalid("AUDIENCE_INVALID", "unknown audience")),
        };
        let member_cap = request.member_cap.map(check_cap).transpose()?;
        // A new mode or new lengths make a new plan; lengths left out keep
        // their current value.
        let mode = match request.mode.as_known() {
            Some(StudyMode::STUDY_MODE_UNSPECIFIED) => current.timer().plan.mode(),
            Some(mode) => mode,
            None => return Err(AppError::invalid("STUDY_MODE_REQUIRED", "unknown timer mode")),
        };
        let defaults = match current.timer().plan {
            Plan::Pomodoro(cycle) => cycle,
            Plan::Open => settings::of(core, user_id).await?.pomodoro(),
        };
        let plan = Self::plan(mode, request.pomodoro.as_option(), defaults)?;

        let mut tx = core.db.begin().await?;
        let mut room = lock(&mut tx, room_id).await?.ok_or_else(not_found)?;
        ensure_open(&room)?;
        if let Some(title) = title {
            room.title = title;
        }
        if let Some(audience) = audience {
            room.audience = audience;
        }
        if let Some(member_cap) = member_cap {
            room.member_cap = member_cap;
        }
        sqlx::query("UPDATE school_rooms SET title = $2, audience = $3, member_cap = $4 WHERE id = $1")
            .bind(room.id)
            .bind(&room.title)
            .bind(room.audience)
            .bind(room.member_cap)
            .execute(&mut *tx)
            .await?;
        let before = room.timer();
        let replanned = plan != before.plan;
        if replanned {
            // What everyone studied under the old lengths is counted first.
            self.count_members(&mut tx, &room, &before, at).await?;
            let mut timer = before;
            timer.replan(plan, at);
            store_timer(&mut tx, &mut room, &timer, at).await?;
        }
        tx.commit().await?;

        if let Ok(mut rooms) = self.room_protos(std::slice::from_ref(&room), user_id, at, Some(0)).await
            && let Some(mut proto) = rooms.pop()
        {
            // The topic's event is the same for everyone listening.
            proto.joined = false;
            proto.invited = false;
            proto.invitees.clear();
            self.push_room(room.id, Push::RoomUpdated(Box::new(proto))).await;
        }
        if replanned {
            self.push_room(room.id, Push::RoomTimer(Box::new(room.timer().snapshot(at)))).await;
        }
        // Public > friends > invited only: only a narrower audience leaves someone out.
        let reach = |audience: i16| match audience {
            AUDIENCE_PUBLIC => 2,
            AUDIENCE_FRIENDS => 1,
            _ => 0,
        };
        if reach(room.audience) < reach(current.audience) {
            // Whoever the new audience leaves out stops following the room.
            self.core.rt.recheck_topic(&topic(room.id)).await;
        }
        Ok(room)
    }

    /// Counts the study time of everyone in a room up to `at` under `timer`
    /// — before the timer changes for all of them. Their heartbeats are not
    /// touched: a member who has in fact gone is still cut back to theirs.
    pub(super) async fn count_members(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        room: &RoomRow,
        timer: &Timer,
        at: Time,
    ) -> AppResult<()> {
        for mut session in member_sessions(tx, room.id, true).await? {
            sessions::advance(tx, &mut session, timer, at).await?;
        }
        Ok(())
    }

    /// Enters a room. Whatever session the caller had going ends first.
    pub(crate) async fn join_room(
        &self,
        user_id: Uuid,
        room_id: Uuid,
        subject_id: &str,
    ) -> AppResult<(RoomRow, SessionRow)> {
        let core = &self.core;
        let at = self.now();
        let subject = subjects::require_opt(core, user_id, subject_id).await?;
        let room = find_room(core, room_id).await?;
        // Whether the room exists for the caller comes first, then whether it is open.
        ensure!(
            self.may_see(&room, user_id).await? || (!room.is_open() && room.host_id == user_id),
            not_found,
            "ROOM_NOT_FOUND",
            "study room not found"
        );
        ensure_open(&room)?;
        core.cache.rate_limit(&format!("school:room-join:{user_id}"), 30, StdDuration::from_secs(60)).await?;

        // Already inside: nothing to do.
        let inside: Option<SessionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM school_study_sessions WHERE user_id = $1 AND room_id = $2 AND state <> {ENDED}",
            sessions::COLUMNS
        )))
        .bind(user_id)
        .bind(room_id)
        .fetch_optional(&core.db)
        .await?;
        if let Some(session) = inside.filter(|session| !self.is_overdue(session, at)) {
            return Ok((room, session));
        }
        ensure!(
            room.member_count < room.member_cap || room.host_id == user_id,
            precondition,
            "ROOM_FULL",
            "the room is full"
        );
        if let Some(session) = self.end_current_session(user_id, room_id, at).await? {
            return Ok((room, session));
        }

        let mut tx = core.db.begin().await?;
        crate::lock_user(&mut tx, "session", user_id).await?;
        // Another request of the caller's may have got here first: entering
        // twice at once is entering once. (A session started elsewhere in
        // that instant is not ended from here — that would take two rooms'
        // locks — so the caller is told and tries again.)
        let going: Option<SessionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM school_study_sessions WHERE user_id = $1 AND state <> {ENDED}",
            sessions::COLUMNS
        )))
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;
        match going {
            Some(session) if session.room_id == Some(room_id) => return Ok((room, session)),
            Some(_) => return Err(sessions::already_active()),
            None => {}
        }
        let mut room = lock(&mut tx, room_id).await?.ok_or_else(not_found)?;
        ensure_open(&room)?;
        // The host may have removed the caller since the check above:
        // removals are made under this lock, so this answer stands.
        let banned: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM school_room_bans WHERE room_id = $1 AND user_id = $2)")
                .bind(room.id)
                .bind(user_id)
                .fetch_one(&mut *tx)
                .await?;
        ensure!(!banned, not_found, "ROOM_NOT_FOUND", "study room not found");
        let members = member_sessions(&mut tx, room.id, false).await?;
        ensure!(
            (members.len() as i32) < room.member_cap || room.host_id == user_id,
            precondition,
            "ROOM_FULL",
            "the room is full"
        );
        let new = NewSession {
            user_id,
            subject_id: subject.as_ref().map(|s| s.id),
            memo: "",
            source: SOURCE_ROOM,
            plan: Plan::Open,
            room: Some((room.id, at)),
            at,
        };
        let session = sessions::insert(&mut tx, &new).await?;
        sqlx::query("INSERT INTO school_room_visits (room_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(room.id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        room.member_count = members.len() as i32 + 1;
        room.empty_since = None;
        sqlx::query("UPDATE school_rooms SET member_count = $2, empty_since = NULL WHERE id = $1")
            .bind(room.id)
            .bind(room.member_count)
            .execute(&mut *tx)
            .await?;
        // With the host away the timer runs by itself, so nobody waits for them.
        let host_present = user_id == room.host_id || members.iter().any(|member| member.user_id == room.host_id);
        let before = room.timer();
        let mut timer = before;
        if !host_present {
            timer.run_from(at);
        }
        if timer != before {
            store_timer(&mut tx, &mut room, &timer, at).await?;
        }
        tx.commit().await?;

        // A block or an ended friendship is not decided under the room's
        // lock: if one landed while the caller was on their way in, its
        // cleanup may have looked before they were inside. Look again now
        // that they are, and show them out if they no longer belong.
        if !self.may_come_in(&room, user_id).await? {
            self.remove_from_room(room.id, user_id, false, at).await?;
            return Err(not_found());
        }

        self.announce_entered(&room, &session, at).await;
        if timer != before {
            self.push_room(room.id, Push::RoomTimer(Box::new(timer.snapshot(at)))).await;
        }
        Ok((room, session))
    }

    /// Leaves the room the caller is in; the session ends and is logged.
    pub(crate) async fn leave_room(&self, user_id: Uuid) -> AppResult<StudySession> {
        let at = self.now();
        let mut tx = self.core.db.begin().await?;
        let gone = match self.active_session(&mut tx, user_id, at).await? {
            Active::Live(session, room) if session.in_room() => {
                self.end_locked(&mut tx, session, room, End::Client(at), Leave::Left, at).await?
            }
            // The server gave up on the stay already: it is over all the same.
            Active::Expired(gone) if gone.exit.is_some() => gone,
            Active::Expired(gone) => {
                tx.commit().await?;
                self.announce_gone(&gone, at).await;
                return Err(not_in_room());
            }
            _ => return Err(not_in_room()),
        };
        tx.commit().await?;
        self.announce_gone(&gone, at).await;
        // Someone who may not come back in (a room they were let into by
        // an invitation now used up, say) stops following it.
        if let Some(room_id) = gone.exit.as_ref().map(|exit| exit.room_id) {
            self.core.rt.revoke(user_id, &topic(room_id)).await;
        }
        let session = &gone.ended.session;
        Ok(session.to_proto(gone.ended.subject.as_ref(), &session.own_timer(), at, self.idle_limit()))
    }

    /// Changes what the caller studies in their room: the time so far is
    /// logged under the old subject and a new session starts under the new.
    pub(crate) async fn set_room_subject(&self, user_id: Uuid, subject_id: &str) -> AppResult<StudySession> {
        let core = &self.core;
        let at = self.now();
        let subject = subjects::require_opt(core, user_id, subject_id).await?;
        let mut tx = core.db.begin().await?;
        let (old, room) = match self.active_session(&mut tx, user_id, at).await? {
            Active::Live(session, Some(room)) if session.in_room() => (session, room),
            Active::Expired(gone) => {
                tx.commit().await?;
                self.announce_gone(&gone, at).await;
                return Err(not_in_room());
            }
            _ => return Err(not_in_room()),
        };
        let joined_at = old.room_joined_at.unwrap_or(old.started_at);
        let timer = room.timer();
        let ended = self.finish(&mut tx, old, &timer, End::Client(at)).await?;
        let new = NewSession {
            user_id,
            subject_id: subject.as_ref().map(|s| s.id),
            memo: "",
            source: SOURCE_ROOM,
            plan: Plan::Open,
            room: Some((room.id, joined_at)),
            at,
        };
        let session = sessions::insert(&mut tx, &new).await?;
        tx.commit().await?;

        // The stretch under the old subject is logged; the user's devices
        // only need the session that goes on.
        self.track_logged(&ended.session).await;
        let proto = session.to_proto(subject.as_ref(), &timer, at, self.idle_limit());
        self.push(user_id, Push::Session(Box::new(proto.clone()))).await;
        if let Ok(member) = self.member_event(&session, &room, at).await {
            self.push_room(room.id, Push::RoomMemberUpdated(Box::new(member))).await;
        }
        self.announce_studying_quietly(user_id, at).await;
        Ok(proto)
    }

    /// Invites friends of the host; returns everyone invited to the room now.
    pub(crate) async fn invite_to_room(
        &self,
        user_id: Uuid,
        room_id: Uuid,
        ids: &[String],
    ) -> AppResult<Vec<UserSummary>> {
        let core = &self.core;
        let at = self.now();
        let room = self.hosted_room(room_id, user_id).await?;
        ensure_open(&room)?;
        ensure!(!ids.is_empty(), invalid, "NO_USERS", "say who to invite");
        let invitees = self.check_invitees(user_id, ids).await?;
        core.cache.rate_limit(&format!("school:room-invite:{user_id}"), 60, StdDuration::from_secs(3600)).await?;
        let host = tokuly_identity::summary(core, user_id).await?;

        let mut tx = core.db.begin().await?;
        let room = lock(&mut tx, room_id).await?.ok_or_else(not_found)?;
        ensure_open(&room)?;
        let new = self.add_invites(&mut tx, &room, &host, &invitees).await?;
        tx.commit().await?;
        self.announce_invites(&room, &new, at).await;

        let invited: Vec<Uuid> = sqlx::query_scalar(
            "SELECT user_id FROM school_room_invites WHERE room_id = $1 AND withdrawn_at IS NULL ORDER BY created_at, user_id",
        )
        .bind(room_id)
        .fetch_all(&core.db)
        .await?;
        let users = tokuly_identity::summaries(core, &invited).await?;
        Ok(invited.iter().filter_map(|id| users.get(id).filter(|user| !user.deleted).cloned()).collect())
    }

    /// Withdraws an invitation. The row stays, marked: the invitee was shown
    /// the room, and can still report it.
    pub(crate) async fn uninvite_from_room(&self, user_id: Uuid, room_id: Uuid, target: Uuid) -> AppResult<()> {
        let room = self.hosted_room(room_id, user_id).await?;
        withdraw_invite(&self.core.db, room.id, target, self.now()).await?;
        self.core.rt.revoke(target, &topic(room.id)).await;
        Ok(())
    }

    /// Host: removes a member for good.
    pub(crate) async fn remove_member(&self, user_id: Uuid, room_id: Uuid, target: Uuid) -> AppResult<()> {
        let room = self.hosted_room(room_id, user_id).await?;
        ensure!(target != user_id, invalid, "CANNOT_REMOVE_SELF", "leave the room instead of removing yourself");
        let visited: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM school_room_visits WHERE room_id = $1 AND user_id = $2)")
                .bind(room.id)
                .bind(target)
                .fetch_one(&self.core.db)
                .await?;
        // Someone who is inside, or was (everyone who enters leaves a visit):
        // out if they are in, and kept out from now on.
        ensure!(visited, not_found, "USER_NOT_IN_ROOM", "this person has not been in the room");
        self.remove_from_room(room.id, target, true, self.now()).await?;
        Ok(())
    }

    /// Takes `target` out of a room if they are in it, and withdraws their
    /// invitation. With `ban` they cannot come (back) in. Returns whether
    /// they were inside.
    pub(crate) async fn remove_from_room(&self, room_id: Uuid, target: Uuid, ban: bool, at: Time) -> AppResult<bool> {
        let mut tx = self.core.db.begin().await?;
        let Some(mut room) = lock(&mut tx, room_id).await? else { return Ok(false) };
        withdraw_invite(&mut *tx, room_id, target, at).await?;
        let session: Option<SessionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM school_study_sessions WHERE room_id = $1 AND user_id = $2 AND state <> {ENDED} FOR UPDATE",
            sessions::COLUMNS
        )))
        .bind(room_id)
        .bind(target)
        .fetch_optional(&mut *tx)
        .await?;
        let gone = match session {
            Some(session) => {
                let (ended, exit) =
                    self.depart(&mut tx, &mut room, session, End::Server(at), Leave::Removed, at).await?;
                Some(Gone { ended, exit: Some(exit) })
            }
            None => None,
        };
        if ban {
            sqlx::query("INSERT INTO school_room_bans (room_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(room_id)
                .bind(target)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        if let Some(gone) = &gone {
            self.announce_gone(gone, at).await;
        }
        // Removed (and maybe banned), or invited no longer: the guard decides.
        self.core.rt.revoke(target, &topic(room_id)).await;
        Ok(gone.is_some())
    }

    pub(crate) async fn list_rooms(
        &self,
        viewer: Uuid,
        scope: RoomScope,
        page: &Page,
    ) -> AppResult<(Vec<StudyRoom>, PageInfo)> {
        let core = &self.core;
        let at = self.now();
        if scope == RoomScope::ROOM_SCOPE_MINE {
            // The room the caller hosts and the one they are in: two at most.
            let rooms: Vec<RoomRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {COLUMNS} FROM school_rooms r
                 WHERE r.closed_at IS NULL
                   AND (r.host_id = $1 OR r.id IN (SELECT room_id FROM school_study_sessions WHERE user_id = $1 AND state <> {ENDED}))
                 ORDER BY r.id DESC"
            )))
            .bind(viewer)
            .fetch_all(&core.db)
            .await?;
            return Ok((self.room_protos(&rooms, viewer, at, Some(MEMBER_PREVIEW)).await?, PageInfo::default()));
        }
        let hidden = graph::blocked_ids_either(core, viewer).await?;
        // What the listings of other people's rooms leave out: closed and
        // empty rooms, rooms the viewer was removed from, and rooms of or
        // with people in a block with them.
        let listed = format!(
            "r.closed_at IS NULL AND r.host_id <> ALL($2)
             AND NOT EXISTS (SELECT 1 FROM school_room_bans b WHERE b.room_id = r.id AND b.user_id = $1)
             AND NOT EXISTS (SELECT 1 FROM school_study_sessions m WHERE m.room_id = r.id AND m.state <> {ENDED} AND m.user_id = ANY($2))"
        );
        let cursor: Option<(i32, Uuid)> = page.decode()?;
        let mut rooms: Vec<RoomRow> = match scope {
            RoomScope::ROOM_SCOPE_PUBLIC => {
                sqlx::query_as(sqlx::AssertSqlSafe(format!(
                    "SELECT {COLUMNS} FROM school_rooms r
                     WHERE {listed} AND r.audience = {AUDIENCE_PUBLIC} AND r.member_count > 0
                       AND ($3::int IS NULL OR (r.member_count, r.id) < ($3, $4))
                     ORDER BY r.member_count DESC, r.id DESC LIMIT $5"
                )))
                .bind(viewer)
                .bind(&hidden)
                .bind(cursor.map(|(count, _)| count))
                .bind(cursor.map(|(_, id)| id))
                .bind(page.fetch())
                .fetch_all(&core.db)
                .await?
            }
            RoomScope::ROOM_SCOPE_FRIENDS => {
                let friends = graph::friend_ids(core, viewer).await?;
                sqlx::query_as(sqlx::AssertSqlSafe(format!(
                    "SELECT {COLUMNS} FROM school_rooms r
                     WHERE {listed} AND r.member_count > 0 AND r.host_id <> $1
                       AND ((r.host_id = ANY($6) AND r.audience IN ({AUDIENCE_PUBLIC}, {AUDIENCE_FRIENDS}))
                            OR EXISTS (SELECT 1 FROM school_room_invites i
                                       WHERE i.room_id = r.id AND i.user_id = $1 AND i.withdrawn_at IS NULL))
                       AND ($3::int IS NULL OR (r.member_count, r.id) < ($3, $4))
                     ORDER BY r.member_count DESC, r.id DESC LIMIT $5"
                )))
                .bind(viewer)
                .bind(&hidden)
                .bind(cursor.map(|(count, _)| count))
                .bind(cursor.map(|(_, id)| id))
                .bind(page.fetch())
                .bind(&friends)
                .fetch_all(&core.db)
                .await?
            }
            RoomScope::ROOM_SCOPE_MINE => unreachable!("answered above"),
            RoomScope::ROOM_SCOPE_UNSPECIFIED => {
                return Err(AppError::invalid("ROOM_SCOPE_REQUIRED", "say which rooms: public, friends' or your own"));
            }
        };
        let info = page.finish(&mut rooms, |room| (room.member_count, room.id));
        Ok((self.room_protos(&rooms, viewer, at, Some(MEMBER_PREVIEW)).await?, info))
    }
}

/// Marks an invitation as withdrawn (if there is one in force).
async fn withdraw_invite<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    room_id: Uuid,
    user_id: Uuid,
    at: Time,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE school_room_invites SET withdrawn_at = $3 WHERE room_id = $1 AND user_id = $2 AND withdrawn_at IS NULL",
    )
    .bind(room_id)
    .bind(user_id)
    .bind(at)
    .execute(executor)
    .await?;
    Ok(())
}

fn not_in_room() -> AppError {
    AppError::precondition("NOT_IN_ROOM", "you are not in a study room")
}

async fn find_room(core: &Core, room_id: Uuid) -> AppResult<RoomRow> {
    super::find(core, room_id).await?.ok_or_else(not_found)
}
