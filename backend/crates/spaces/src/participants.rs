//! Inside a room: entering and leaving, raised hands, the stage, co-hosts,
//! handing the room over, muting, removing and banning, emoji bursts and
//! the pin.
//!
//! Each action changes the room under its lock (see [`crate::rooms`]) and
//! afterwards tells the room (realtime) and LiveKit: a role change reaches
//! the person's connection at once, and whoever leaves is disconnected.
use std::time::Duration;

use tokuly_core::{events, outbound, prelude::*};
use tokuly_identity::users;
use tokuly_proto::{common::PageInfo, spaces as pb};

use crate::{
    Spaces,
    events::{ROOM_JOINED, RoomJoined},
    limits::{Limit, RoomFanout},
    model::{PARTICIPANT, ParticipantRow, SPACE, SpaceRow, Venue, leave, role},
    notices,
    rooms::{
        Departed, Tx, already_hosting, depart, ensure_nowhere_else, lock_presence, lock_space, not_host, not_live,
        participant_in_tx, recount, space_ended,
    },
    rules,
};

fn not_in_space() -> AppError {
    AppError::precondition("NOT_IN_SPACE", "enter the room first")
}

fn target_not_in_space() -> AppError {
    AppError::precondition("TARGET_NOT_IN_SPACE", "this person is not in the room")
}

pub(crate) fn not_moderator() -> AppError {
    AppError::denied("NOT_MODERATOR", "only the host and co-hosts can do this")
}

fn cannot_moderate() -> AppError {
    AppError::denied("CANNOT_MODERATE_USER", "you cannot do this to someone of your own rank or above")
}

fn stage_full() -> AppError {
    AppError::precondition("STAGE_FULL", "every seat on stage is taken")
}

fn already_on_stage() -> AppError {
    AppError::precondition("ALREADY_ON_STAGE", "this person is on stage already")
}

fn invalid_user() -> AppError {
    AppError::invalid("INVALID_USER", "this cannot be done to that person")
}

/// The caller's row, if they are inside; `NOT_IN_SPACE` otherwise.
async fn inside(tx: &mut Tx<'_>, space_id: Uuid, user_id: Uuid) -> AppResult<ParticipantRow> {
    participant_in_tx(tx, space_id, user_id).await?.filter(ParticipantRow::is_inside).ok_or_else(not_in_space)
}

/// The caller's row, if they are inside as host or co-host;
/// `NOT_MODERATOR` otherwise.
async fn moderator(tx: &mut Tx<'_>, space_id: Uuid, user_id: Uuid) -> AppResult<ParticipantRow> {
    participant_in_tx(tx, space_id, user_id)
        .await?
        .filter(|row| row.is_inside() && rules::is_moderator(row.role))
        .ok_or_else(not_moderator)
}

/// Someone else's row, if they are inside; `TARGET_NOT_IN_SPACE` otherwise.
async fn target_inside(tx: &mut Tx<'_>, space_id: Uuid, user_id: Uuid) -> AppResult<ParticipantRow> {
    participant_in_tx(tx, space_id, user_id).await?.filter(ParticipantRow::is_inside).ok_or_else(target_not_in_space)
}

/// Gives someone a role; a raised hand and a stage invitation are settled
/// by it either way. The highest role they ever held stays on record.
async fn set_role(tx: &mut Tx<'_>, space_id: Uuid, user_id: Uuid, role: i16) -> AppResult<ParticipantRow> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "UPDATE space_participants p SET role = $3, top_role = GREATEST(p.top_role, $3),
             hand_raised_at = NULL, stage_invited_at = NULL, stage_invited_by = NULL
         WHERE p.space_id = $1 AND p.user_id = $2
         RETURNING {PARTICIPANT}"
    )))
    .bind(space_id)
    .bind(user_id)
    .bind(role)
    .fetch_one(&mut **tx)
    .await?)
}

impl Spaces {
    /// Announces someone on stage or with a raised hand to the room.
    pub(crate) async fn push_participant(&self, row: &ParticipantRow) {
        match self.render_participant(row, false).await {
            Ok(participant) => {
                let updated = pb::SpaceParticipantUpdated {
                    space_id: row.space_id.to_string(),
                    participant: participant.into(),
                    ..Default::default()
                };
                self.push_topic(row.space_id, updated).await;
            }
            Err(e) => tracing::warn!(space_id = %row.space_id, error = %e, "participant push failed"),
        }
    }

    /// Whether `user_id` is the host, or inside as a co-host.
    pub(crate) async fn is_moderator(&self, space: &SpaceRow, user_id: Uuid) -> AppResult<bool> {
        if space.host_id == user_id {
            return Ok(true);
        }
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM space_participants
                            WHERE space_id = $1 AND user_id = $2 AND left_at IS NULL AND role >= $3)",
        )
        .bind(space.id)
        .bind(user_id)
        .bind(role::COHOST)
        .fetch_one(&self.core.db)
        .await?)
    }

    /// `NOT_MODERATOR` unless `user_id` is the host or inside as a co-host.
    pub(crate) async fn require_moderator(&self, space: &SpaceRow, user_id: Uuid) -> AppResult<()> {
        if self.is_moderator(space, user_id).await? { Ok(()) } else { Err(not_moderator()) }
    }

    async fn is_banned(&self, space_id: Uuid, user_id: Uuid) -> AppResult<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM space_bans WHERE space_id = $1 AND user_id = $2)")
            .bind(space_id)
            .bind(user_id)
            .fetch_one(&self.core.db)
            .await?)
    }

    async fn is_inside(&self, space_id: Uuid, user_id: Uuid) -> AppResult<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM space_participants WHERE space_id = $1 AND user_id = $2 AND left_at IS NULL)",
        )
        .bind(space_id)
        .bind(user_id)
        .fetch_one(&self.core.db)
        .await?)
    }

    // ───────────────────────────── entering and leaving ─────────────────────────────

    /// Enters a live room. Whoever is inside already (a reconnect, another
    /// device) gets fresh credentials for their current role.
    pub(crate) async fn join_space(
        &self,
        user_id: Uuid,
        space_id: Uuid,
    ) -> AppResult<(SpaceRow, ParticipantRow, pb::AudioCredentials)> {
        let core = &self.core;
        let space = self.visible_space(Some(user_id), space_id).await?;
        if !space.is_live() {
            return Err(not_live());
        }
        self.require_audio()?;
        let banned = || AppError::denied("BANNED_FROM_SPACE", "you cannot enter this room");
        // Checked before anything is given up for this room, and again
        // under the room's lock.
        if self.is_banned(space_id, user_id).await? {
            return Err(banned());
        }
        self.limit(Limit::Enter, user_id).await?;
        let me = tokuly_identity::summary(core, user_id).await?;
        let venue = Venue::Space(space_id);
        if space.listener_count == 0 || space.host_id == user_id {
            // LiveKit may have closed a room nobody was connected to.
            self.open_room(venue).await?;
        }

        // What the transaction did: someone already inside asked again, or
        // someone entered (for the first time, or coming back).
        enum Entered {
            Again(SpaceRow, ParticipantRow),
            Now(SpaceRow, ParticipantRow),
        }
        let enter = || async {
            let mut tx = core.db.begin().await?;
            lock_presence(&mut tx, user_id).await?;
            let locked = lock_space(&mut tx, space_id).await?;
            if !locked.is_live() {
                return Err(not_live());
            }
            let banned_meanwhile: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM space_bans WHERE space_id = $1 AND user_id = $2)")
                    .bind(space_id)
                    .bind(user_id)
                    .fetch_one(&mut *tx)
                    .await?;
            if banned_meanwhile {
                return Err(banned());
            }

            let existing = participant_in_tx(&mut tx, space_id, user_id).await?;
            if let Some(row) = existing.as_ref().filter(|row| row.is_inside()) {
                // Asking again is a sign of life: the time to (re)connect starts over.
                sqlx::query(
                    "UPDATE space_participants SET seen_at = now(), missing_since = NULL WHERE space_id = $1 AND user_id = $2",
                )
                .bind(space_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
                return Ok(Entered::Again(locked, row.clone()));
            }

            let role = if user_id == locked.host_id {
                role::HOST
            } else if existing.as_ref().is_some_and(|row| row.role == role::COHOST) {
                role::COHOST
            } else {
                role::LISTENER
            };
            ensure!(
                rules::is_moderator(role) || locked.listener_count < self.cfg.max_listeners,
                precondition,
                "SPACE_FULL",
                "this room is full"
            );
            // Nothing stands in the way but being somewhere else: one room
            // at a time.
            ensure_nowhere_else(&mut tx, user_id, venue).await?;
            let row: ParticipantRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "INSERT INTO space_participants AS p (space_id, user_id, role, top_role) VALUES ($1, $2, $3, $3)
                 ON CONFLICT (space_id, user_id) DO UPDATE SET
                     role = EXCLUDED.role, top_role = GREATEST(p.top_role, EXCLUDED.role), joined_at = now(),
                     left_at = NULL, leave_reason = 0, seen_at = NULL, missing_since = NULL, hand_raised_at = NULL,
                     stage_invited_at = NULL, stage_invited_by = NULL
                 RETURNING {PARTICIPANT}"
            )))
            .bind(space_id)
            .bind(user_id)
            .bind(role)
            .fetch_one(&mut *tx)
            .await?;
            let space = recount(&mut tx, space_id).await?;
            if existing.is_none() && role != role::HOST {
                let joined =
                    RoomJoined { space_id, user_id, host_id: space.host_id, audience: i32::from(space.audience) };
                events::emit(&mut *tx, ROOM_JOINED, Some(user_id), &joined).await?;
            }
            tx.commit().await?;
            Ok(Entered::Now(space, row))
        };
        let (space, row) = match self.entering(user_id, venue, enter).await? {
            Entered::Again(space, row) => (space, row),
            Entered::Now(space, row) => {
                if rules::on_stage(row.role) {
                    self.push_participant(&row).await;
                }
                self.push_counts(&space).await;
                (space, row)
            }
        };
        let credentials = self.credentials(venue, user_id, &notices::name_of(&me), row.role)?;
        Ok((space, row, credentials))
    }

    /// Takes people out of a room (they left, dropped off, were removed)
    /// and tells the room and LiveKit. Returns who actually was inside.
    pub(crate) async fn remove_from_space(
        &self,
        space_id: Uuid,
        user_ids: &[Uuid],
        reason: i16,
    ) -> AppResult<Vec<Departed>> {
        let mut tx = self.core.db.begin().await?;
        match lock_space(&mut tx, space_id).await {
            Ok(_) => {}
            // Purged in the meantime: nobody is inside any more.
            Err(e) if e.is_not_found() => return Ok(Vec::new()),
            Err(e) => return Err(e),
        }
        let departed = depart(&mut tx, space_id, Some(user_ids), reason).await?;
        if departed.is_empty() {
            return Ok(departed);
        }
        let space = recount(&mut tx, space_id).await?;
        tx.commit().await?;
        self.after_departures(&space, &departed, reason).await;
        Ok(departed)
    }

    /// What follows people leaving a room, once that is committed: the
    /// room hears of those who were on stage or had a hand up, people who
    /// did not leave by themselves are told, and LiveKit disconnects them.
    pub(crate) async fn after_departures(&self, space: &SpaceRow, departed: &[Departed], reason: i16) {
        let venue = Venue::Space(space.id);
        for person in departed {
            let left = pb::SpaceParticipantLeft {
                space_id: space.id.to_string(),
                user_id: person.user_id.to_string(),
                reason: i32::from(reason).into(),
                ..Default::default()
            };
            if rules::on_stage(person.role) || person.hand_raised {
                self.push_topic(space.id, left.clone()).await;
            }
            if reason != leave::LEFT {
                self.push_user(person.user_id, left).await;
            }
            // Someone whose connection is gone needs no disconnecting.
            if reason != leave::DISCONNECTED {
                self.kick(venue, person.user_id).await;
            }
        }
        self.push_counts(space).await;
    }

    /// Leaves a room. Leaving a room one is not in succeeds, as long as
    /// the room is one the caller may see.
    pub(crate) async fn leave_space(&self, user_id: Uuid, space_id: Uuid) -> AppResult<()> {
        if self.find_space(space_id).await?.is_none() {
            return Err(crate::rooms::space_not_found());
        }
        if self.remove_from_space(space_id, &[user_id], leave::LEFT).await?.is_empty() {
            self.visible_space(Some(user_id), space_id).await?;
        }
        Ok(())
    }

    /// Takes `user_id` out of every room and hangout they are inside,
    /// except `staying`: a person is in one place at a time.
    pub(crate) async fn leave_everything(&self, user_id: Uuid, staying: Option<Venue>) -> AppResult<()> {
        let db = &self.core.db;
        let spaces: Vec<Uuid> =
            sqlx::query_scalar("SELECT space_id FROM space_participants WHERE user_id = $1 AND left_at IS NULL")
                .bind(user_id)
                .fetch_all(db)
                .await?;
        for space_id in spaces {
            if staying != Some(Venue::Space(space_id)) {
                self.remove_from_space(space_id, &[user_id], leave::LEFT).await?;
            }
        }
        let hangouts: Vec<Uuid> = sqlx::query_scalar("SELECT hangout_id FROM hangout_presence WHERE user_id = $1")
            .bind(user_id)
            .fetch_all(db)
            .await?;
        for hangout_id in hangouts {
            if staying != Some(Venue::Hangout(hangout_id)) {
                self.remove_from_hangout_voice(hangout_id, &[user_id], true).await?;
            }
        }
        Ok(())
    }

    /// The people inside a room: stage, raised hands, the rest. Or, with
    /// the PAST filter, the people who are gone — the way to find someone
    /// to report or block after they left.
    pub(crate) async fn list_participants(
        &self,
        viewer: Uuid,
        space_id: Uuid,
        filter: Option<pb::ParticipantFilter>,
        page: &Page,
    ) -> AppResult<(Vec<pb::SpaceParticipant>, PageInfo)> {
        let space = self.visible_space(Some(viewer), space_id).await?;
        self.limit(Limit::Discover, viewer).await?;
        let moderates = self.is_moderator(&space, viewer).await?;
        // The order moves with every hand and arrival, so pages are cut by position.
        let offset = page.decode::<i64>()?.unwrap_or(0).max(0);
        let mut rows: Vec<ParticipantRow> = if filter == Some(pb::ParticipantFilter::PARTICIPANT_FILTER_PAST) {
            // Who was on stage is everyone's to know; who merely listened
            // is for the moderators.
            sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {PARTICIPANT} FROM space_participants p
                 WHERE p.space_id = $1 AND p.left_at IS NOT NULL AND ($2 OR p.top_role >= $3)
                 ORDER BY p.left_at DESC, p.user_id
                 LIMIT $4 OFFSET $5"
            )))
            .bind(space_id)
            .bind(moderates)
            .bind(role::SPEAKER)
            .bind(page.fetch())
            .bind(offset)
            .fetch_all(&self.core.db)
            .await?
        } else {
            let (min_role, max_role, hands_only) = match filter {
                Some(pb::ParticipantFilter::PARTICIPANT_FILTER_STAGE) => (role::SPEAKER, role::HOST, false),
                Some(pb::ParticipantFilter::PARTICIPANT_FILTER_HANDS) => (role::LISTENER, role::LISTENER, true),
                Some(pb::ParticipantFilter::PARTICIPANT_FILTER_AUDIENCE) => (role::LISTENER, role::LISTENER, false),
                _ => (role::LISTENER, role::HOST, false),
            };
            sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {PARTICIPANT} FROM space_participants p
                 WHERE p.space_id = $1 AND p.left_at IS NULL AND p.role BETWEEN $2 AND $3
                   AND (NOT $4 OR p.hand_raised_at IS NOT NULL)
                 ORDER BY (p.role >= {speaker}) DESC, CASE WHEN p.role >= {speaker} THEN -p.role ELSE 0 END,
                          (p.hand_raised_at IS NULL), COALESCE(p.hand_raised_at, p.joined_at), p.joined_at, p.user_id
                 LIMIT $5 OFFSET $6",
                speaker = role::SPEAKER
            )))
            .bind(space_id)
            .bind(min_role)
            .bind(max_role)
            .bind(hands_only)
            .bind(page.fetch())
            .bind(offset)
            .fetch_all(&self.core.db)
            .await?
        };
        let info = page.finish(&mut rows, |_| offset + page.limit);
        Ok((self.render_participants(&rows, viewer, moderates).await?, info))
    }

    // ───────────────────────────── hands and the stage ─────────────────────────────

    /// Raises or lowers the caller's hand.
    pub(crate) async fn set_hand_raised(
        &self,
        user_id: Uuid,
        space_id: Uuid,
        raised: bool,
    ) -> AppResult<ParticipantRow> {
        self.visible_space(Some(user_id), space_id).await?;
        self.limit(Limit::Hand, user_id).await?;
        let mut tx = self.core.db.begin().await?;
        let space = lock_space(&mut tx, space_id).await?;
        if !space.is_live() {
            return Err(not_live());
        }
        let me = inside(&mut tx, space_id, user_id).await?;
        if raised && rules::on_stage(me.role) {
            return Err(already_on_stage());
        }
        if raised == me.hand_raised_at.is_some() {
            return Ok(me);
        }
        let row: ParticipantRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE space_participants p SET hand_raised_at = CASE WHEN $3 THEN now() END
             WHERE p.space_id = $1 AND p.user_id = $2
             RETURNING {PARTICIPANT}"
        )))
        .bind(space_id)
        .bind(user_id)
        .bind(raised)
        .fetch_one(&mut *tx)
        .await?;
        let space = recount(&mut tx, space_id).await?;
        tx.commit().await?;
        // A large room waving its hands hears the count, not every hand.
        if self.room_budget(space_id, RoomFanout::Hands).await {
            self.push_participant(&row).await;
        }
        self.push_counts(&space).await;
        Ok(row)
    }

    /// Brings a listener on stage: at once if their hand is up, by
    /// invitation otherwise. Returns their row and whether they are a
    /// speaker now.
    pub(crate) async fn invite_to_stage(
        &self,
        actor: Uuid,
        space_id: Uuid,
        target: Uuid,
    ) -> AppResult<(ParticipantRow, bool)> {
        self.visible_space(Some(actor), space_id).await?;
        self.limit(Limit::Moderate, actor).await?;
        // Being called on stage interrupts whoever it happens to: not the
        // same person over and over.
        self.limit_stage_invitation(actor, target).await?;
        let mut tx = self.core.db.begin().await?;
        let space = lock_space(&mut tx, space_id).await?;
        if !space.is_live() {
            return Err(not_live());
        }
        moderator(&mut tx, space_id, actor).await?;
        let listener = target_inside(&mut tx, space_id, target).await?;
        if rules::on_stage(listener.role) {
            return Err(already_on_stage());
        }
        if space.speaker_count >= self.cfg.max_speakers {
            return Err(stage_full());
        }
        if listener.hand_raised_at.is_some() {
            let row = set_role(&mut tx, space_id, target, role::SPEAKER).await?;
            let space = recount(&mut tx, space_id).await?;
            tx.commit().await?;
            self.after_role_change(&space, &row).await;
            return Ok((row, true));
        }
        let row: ParticipantRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE space_participants p SET stage_invited_at = now(), stage_invited_by = $3
             WHERE p.space_id = $1 AND p.user_id = $2
             RETURNING {PARTICIPANT}"
        )))
        .bind(space_id)
        .bind(target)
        .bind(actor)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        self.push_stage_invitation(space_id, target, actor, true).await;
        Ok((row, false))
    }

    async fn push_stage_invitation(&self, space_id: Uuid, target: Uuid, by: Uuid, invited: bool) {
        match tokuly_identity::summary(&self.core, by).await {
            Ok(by) => {
                let invitation = pb::StageInvitation {
                    space_id: space_id.to_string(),
                    invited,
                    invited_by: by.into(),
                    ..Default::default()
                };
                self.push_user(target, invitation).await;
            }
            Err(e) => tracing::warn!(%space_id, error = %e, "stage invitation push failed"),
        }
    }

    /// What follows a change of role, once it is committed: the room hears
    /// of it and the person's connection gets the new permissions.
    async fn after_role_change(&self, space: &SpaceRow, row: &ParticipantRow) {
        self.push_participant(row).await;
        self.push_counts(space).await;
        self.apply_role(Venue::Space(space.id), row.user_id, row.role).await;
    }

    /// Accepts or declines an invitation on stage.
    pub(crate) async fn respond_to_stage_invite(
        &self,
        user_id: Uuid,
        space_id: Uuid,
        accept: bool,
    ) -> AppResult<ParticipantRow> {
        self.visible_space(Some(user_id), space_id).await?;
        let mut tx = self.core.db.begin().await?;
        let space = lock_space(&mut tx, space_id).await?;
        if !space.is_live() {
            return Err(not_live());
        }
        let me = inside(&mut tx, space_id, user_id).await?;
        ensure!(me.stage_invited_at.is_some(), precondition, "NO_STAGE_INVITATION", "nobody invited you on stage");
        if !accept {
            let row = set_role(&mut tx, space_id, user_id, me.role).await?;
            let space = recount(&mut tx, space_id).await?;
            tx.commit().await?;
            if me.hand_raised_at.is_some() {
                // Declining also takes the hand down.
                self.push_participant(&row).await;
                self.push_counts(&space).await;
            }
            return Ok(row);
        }
        if space.speaker_count >= self.cfg.max_speakers {
            return Err(stage_full());
        }
        let row = set_role(&mut tx, space_id, user_id, role::SPEAKER).await?;
        let space = recount(&mut tx, space_id).await?;
        tx.commit().await?;
        self.after_role_change(&space, &row).await;
        Ok(row)
    }

    /// Moves a speaker back to the audience; `target` = `None` is the
    /// caller stepping down. On a listener it withdraws a stage invitation
    /// and dismisses a raised hand.
    pub(crate) async fn move_to_audience(
        &self,
        actor: Uuid,
        space_id: Uuid,
        target: Option<Uuid>,
    ) -> AppResult<ParticipantRow> {
        self.visible_space(Some(actor), space_id).await?;
        if target.is_some_and(|target| target != actor) {
            self.limit(Limit::Moderate, actor).await?;
        }
        let mut tx = self.core.db.begin().await?;
        let space = lock_space(&mut tx, space_id).await?;
        if !space.is_live() {
            return Err(not_live());
        }
        let me = inside(&mut tx, space_id, actor).await?;
        let before = match target.filter(|target| *target != actor) {
            None => {
                ensure!(
                    me.role != role::HOST,
                    precondition,
                    "HOST_CANNOT_STEP_DOWN",
                    "hand the room over or end it instead"
                );
                if !rules::on_stage(me.role) {
                    return Ok(me);
                }
                me
            }
            Some(target) => {
                if !rules::is_moderator(me.role) {
                    return Err(not_moderator());
                }
                let other = target_inside(&mut tx, space_id, target).await?;
                if rules::on_stage(other.role) && !rules::may_moderate(me.role, other.role) {
                    return Err(cannot_moderate());
                }
                other
            }
        };
        let row = set_role(&mut tx, space_id, before.user_id, role::LISTENER).await?;
        let space = recount(&mut tx, space_id).await?;
        tx.commit().await?;

        if rules::on_stage(before.role) {
            self.after_role_change(&space, &row).await;
        } else if before.hand_raised_at.is_some() {
            self.push_participant(&row).await;
            self.push_counts(&space).await;
        }
        if before.stage_invited_at.is_some() && before.user_id != actor {
            self.push_stage_invitation(space_id, before.user_id, actor, false).await;
        }
        Ok(row)
    }

    // ───────────────────────────── co-hosts and the host ─────────────────────────────

    /// Makes someone inside a co-host, or takes the role away again.
    pub(crate) async fn set_cohost(
        &self,
        host: Uuid,
        space_id: Uuid,
        target: Uuid,
        cohost: bool,
    ) -> AppResult<ParticipantRow> {
        self.visible_space(Some(host), space_id).await?;
        self.limit(Limit::Moderate, host).await?;
        let mut tx = self.core.db.begin().await?;
        let space = lock_space(&mut tx, space_id).await?;
        if !space.is_live() {
            return Err(not_live());
        }
        if space.host_id != host {
            return Err(not_host());
        }
        if target == host {
            return Err(invalid_user());
        }
        let current = participant_in_tx(&mut tx, space_id, target).await?.ok_or_else(target_not_in_space)?;
        let new_role = if cohost {
            if !current.is_inside() {
                return Err(target_not_in_space());
            }
            if current.role == role::COHOST {
                return Ok(current);
            }
            // Co-hosts keep the role while away, so they count either way.
            let cohosts: i64 =
                sqlx::query_scalar("SELECT count(*) FROM space_participants WHERE space_id = $1 AND role = $2")
                    .bind(space_id)
                    .bind(role::COHOST)
                    .fetch_one(&mut *tx)
                    .await?;
            ensure!(
                cohosts < i64::from(self.cfg.max_cohosts),
                precondition,
                "TOO_MANY_COHOSTS",
                "a room has at most {} co-hosts",
                self.cfg.max_cohosts
            );
            if !rules::on_stage(current.role) && space.speaker_count >= self.cfg.max_speakers {
                return Err(stage_full());
            }
            role::COHOST
        } else {
            if current.role != role::COHOST {
                return Ok(current);
            }
            if current.is_inside() { role::SPEAKER } else { role::LISTENER }
        };
        let row = set_role(&mut tx, space_id, target, new_role).await?;
        let space = recount(&mut tx, space_id).await?;
        tx.commit().await?;
        if row.is_inside() {
            self.after_role_change(&space, &row).await;
        }
        Ok(row)
    }

    /// Makes `new_host` (who is inside) the host, inside the transaction
    /// that holds the room's lock; the host so far stays as a co-host.
    /// Follow the commit with [`Self::after_hand_over`].
    pub(crate) async fn hand_over(&self, tx: &mut Tx<'_>, space: &SpaceRow, new_host: Uuid) -> AppResult<SpaceRow> {
        sqlx::query("UPDATE space_participants SET role = $3 WHERE space_id = $1 AND user_id = $2")
            .bind(space.id)
            .bind(space.host_id)
            .bind(role::COHOST)
            .execute(&mut **tx)
            .await?;
        set_role(tx, space.id, new_host, role::HOST).await?;
        sqlx::query("UPDATE spaces SET host_id = $2 WHERE id = $1")
            .bind(space.id)
            .bind(new_host)
            .execute(&mut **tx)
            .await?;
        let handed = recount(tx, space.id).await?;
        if handed.is_public() {
            // The search document names its owner.
            outbound::index(&mut **tx, notices::search_doc(&handed)).await?;
        }
        Ok(handed)
    }

    /// Announces a new host and updates the connections of the two people
    /// who changed role.
    pub(crate) async fn after_hand_over(&self, space: &SpaceRow, previous_host: Uuid) {
        self.push_space(space).await;
        self.push_counts(space).await;
        let venue = Venue::Space(space.id);
        self.apply_role(venue, space.host_id, role::HOST).await;
        self.apply_role(venue, previous_host, role::COHOST).await;
    }

    /// Hands the room to someone inside.
    pub(crate) async fn transfer_host(&self, host: Uuid, space_id: Uuid, target: Uuid) -> AppResult<SpaceRow> {
        self.visible_space(Some(host), space_id).await?;
        self.limit(Limit::Moderate, host).await?;
        let handed: AppResult<SpaceRow> = async {
            let mut tx = self.core.db.begin().await?;
            let space = lock_space(&mut tx, space_id).await?;
            if !space.is_live() {
                return Err(not_live());
            }
            if space.host_id != host {
                return Err(not_host());
            }
            if target == host {
                return Err(invalid_user());
            }
            let successor = target_inside(&mut tx, space_id, target).await?;
            let hosting: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM spaces WHERE host_id = $1 AND state = $2)")
                    .bind(target)
                    .bind(crate::model::state::LIVE)
                    .fetch_one(&mut *tx)
                    .await?;
            if hosting {
                return Err(already_hosting());
            }
            if !rules::on_stage(successor.role) && space.speaker_count >= self.cfg.max_speakers {
                return Err(stage_full());
            }
            let handed = self.hand_over(&mut tx, &space, target).await?;
            tx.commit().await?;
            Ok(handed)
        }
        .await;
        let space = handed.map_err(|e| if e.is_already_exists() { already_hosting() } else { e })?;
        self.after_hand_over(&space, host).await;
        Ok(space)
    }

    // ───────────────────────────── moderation ─────────────────────────────

    /// Mutes a speaker's microphone on the server. Returns how many tracks
    /// were muted.
    pub(crate) async fn mute_speaker(&self, actor: Uuid, space_id: Uuid, target: Uuid) -> AppResult<i32> {
        let space = self.visible_space(Some(actor), space_id).await?;
        if !space.is_live() {
            return Err(not_live());
        }
        self.require_audio()?;
        let roles: Vec<(Uuid, i16)> = sqlx::query_as(
            "SELECT user_id, role FROM space_participants WHERE space_id = $1 AND user_id = ANY($2) AND left_at IS NULL",
        )
        .bind(space_id)
        .bind([actor, target].as_slice())
        .fetch_all(&self.core.db)
        .await?;
        let role_of = |user: Uuid| roles.iter().find(|(id, _)| *id == user).map(|(_, role)| *role);
        let actor_role = role_of(actor).filter(|role| rules::is_moderator(*role)).ok_or_else(not_moderator)?;
        let target_role = role_of(target).ok_or_else(target_not_in_space)?;
        ensure!(rules::on_stage(target_role), precondition, "NOT_ON_STAGE", "only someone on stage can be muted");
        if actor != target && !rules::may_moderate(actor_role, target_role) {
            return Err(cannot_moderate());
        }
        self.limit(Limit::Moderate, actor).await?;
        let muted = self.mute_microphone(Venue::Space(space_id), target).await?;
        let event = pb::SpeakerMuted {
            space_id: space_id.to_string(),
            user_id: target.to_string(),
            muted_by_id: actor.to_string(),
            ..Default::default()
        };
        self.push_topic(space_id, event).await;
        Ok(muted)
    }

    /// Removes someone from the room, and with `ban` keeps them out.
    pub(crate) async fn remove_participant(
        &self,
        actor: Uuid,
        space_id: Uuid,
        target: Uuid,
        ban: bool,
    ) -> AppResult<()> {
        self.visible_space(Some(actor), space_id).await?;
        if target == actor {
            return Err(invalid_user());
        }
        self.limit(Limit::Moderate, actor).await?;
        // Looked up before the room is locked: nothing but the room's own
        // transaction runs while the lock is held.
        let target_exists = !ban || !users::filter_active(&self.core, &[target]).await?.is_empty();
        let mut tx = self.core.db.begin().await?;
        let space = lock_space(&mut tx, space_id).await?;
        if !space.is_live() {
            return Err(not_live());
        }
        let me = moderator(&mut tx, space_id, actor).await?;
        if target == space.host_id {
            return Err(cannot_moderate());
        }
        match participant_in_tx(&mut tx, space_id, target).await? {
            Some(row) if row.is_inside() => {
                if !rules::may_moderate(me.role, row.role) {
                    return Err(cannot_moderate());
                }
            }
            // Someone who left can still be banned, a co-host only by the host.
            Some(row) if ban => {
                if !rules::may_moderate(me.role, row.role) {
                    return Err(cannot_moderate());
                }
            }
            None if ban => {
                if !target_exists {
                    return Err(invalid_user());
                }
            }
            _ => return Err(target_not_in_space()),
        }
        let reason = if ban { leave::BANNED } else { leave::REMOVED };
        let departed = depart(&mut tx, space_id, Some(&[target]), reason).await?;
        if ban {
            sqlx::query(
                "INSERT INTO space_bans (space_id, user_id, banned_by) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            )
            .bind(space_id)
            .bind(target)
            .bind(actor)
            .execute(&mut *tx)
            .await?;
            // Nothing lets them back in: no invitation, no co-host role kept
            // for later. (The invitation stays on record, taken back.)
            sqlx::query(
                "UPDATE space_invites SET revoked_at = now() WHERE space_id = $1 AND user_id = $2 AND revoked_at IS NULL",
            )
            .bind(space_id)
            .bind(target)
            .execute(&mut *tx)
            .await?;
            sqlx::query("UPDATE space_participants SET role = $3 WHERE space_id = $1 AND user_id = $2")
                .bind(space_id)
                .bind(target)
                .bind(role::LISTENER)
                .execute(&mut *tx)
                .await?;
        }
        let space = recount(&mut tx, space_id).await?;
        tx.commit().await?;
        if departed.is_empty() {
            // Banned while away: a stale token must not let them back in.
            self.kick(Venue::Space(space_id), target).await;
        } else {
            self.after_departures(&space, &departed, reason).await;
        }
        Ok(())
    }

    /// Lifts a ban.
    pub(crate) async fn unban_user(&self, actor: Uuid, space_id: Uuid, target: Uuid) -> AppResult<()> {
        let space = self.visible_space(Some(actor), space_id).await?;
        self.require_moderator(&space, actor).await?;
        self.limit(Limit::Moderate, actor).await?;
        sqlx::query("DELETE FROM space_bans WHERE space_id = $1 AND user_id = $2")
            .bind(space_id)
            .bind(target)
            .execute(&self.core.db)
            .await?;
        Ok(())
    }

    /// The people banned from a room, newest first.
    pub(crate) async fn list_bans(
        &self,
        actor: Uuid,
        space_id: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<pb::SpaceBan>, PageInfo)> {
        let space = self.visible_space(Some(actor), space_id).await?;
        self.require_moderator(&space, actor).await?;
        let after: Option<(Time, Uuid)> = page.decode()?;
        let mut rows: Vec<(Uuid, Option<Uuid>, Time)> = sqlx::query_as(
            "SELECT user_id, banned_by, created_at FROM space_bans
             WHERE space_id = $1 AND ($2::timestamptz IS NULL OR (created_at, user_id) < ($2, $3))
             ORDER BY created_at DESC, user_id DESC
             LIMIT $4",
        )
        .bind(space_id)
        .bind(after.map(|(at, _)| at))
        .bind(after.map(|(_, id)| id))
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |(user_id, _, at)| (*at, *user_id));
        let ids: Vec<Uuid> = rows.iter().map(|(user_id, _, _)| *user_id).collect();
        let people = tokuly_identity::summaries(&self.core, &ids).await?;
        let bans = rows
            .iter()
            .map(|(user_id, banned_by, at)| pb::SpaceBan {
                user: users::pick(&people, *user_id).into(),
                banned_by_id: banned_by.map(|id| id.to_string()).unwrap_or_default(),
                banned_at: ts(*at),
                ..Default::default()
            })
            .collect();
        Ok((bans, info))
    }

    // ───────────────────────────── reactions and the pin ─────────────────────────────

    /// Sends an emoji burst to everyone following the room. Not stored.
    pub(crate) async fn send_reaction(&self, user_id: Uuid, space_id: Uuid, emoji: &str) -> AppResult<()> {
        let emoji = rules::emoji(emoji)?;
        let space = self.visible_space(Some(user_id), space_id).await?;
        if !space.is_live() {
            return Err(not_live());
        }
        if !self.is_inside(space_id, user_id).await? {
            return Err(not_in_space());
        }
        self.core
            .cache
            .rate_limit(
                &format!("spaces-react:{space_id}:{user_id}"),
                self.cfg.reactions_per_10s,
                Duration::from_secs(10),
            )
            .await?;
        // A room in full swing passes on what people can watch, not more.
        if self.room_budget(space_id, RoomFanout::Reactions).await {
            let reaction = pb::SpaceReaction {
                space_id: space_id.to_string(),
                user_id: user_id.to_string(),
                emoji,
                ..Default::default()
            };
            self.push_topic(space_id, reaction).await;
        }
        Ok(())
    }

    /// Pins a note, a link or both; both empty unpins.
    pub(crate) async fn set_pin(&self, user_id: Uuid, space_id: Uuid, text: &str, url: &str) -> AppResult<SpaceRow> {
        let (text, url) = rules::pin(text, url)?;
        let space = self.visible_space(Some(user_id), space_id).await?;
        if space.is_over() {
            return Err(space_ended());
        }
        self.require_moderator(&space, user_id).await?;
        self.limit(Limit::Pin, user_id).await?;
        let pinned = !text.is_empty() || !url.is_empty();
        let space: SpaceRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE spaces s SET pin_text = $2, pin_url = $3,
                 pinned_by = CASE WHEN $4 THEN $5::uuid END, pinned_at = CASE WHEN $4 THEN now() END
             WHERE s.id = $1
             RETURNING {SPACE}"
        )))
        .bind(space_id)
        .bind(&text)
        .bind(&url)
        .bind(pinned)
        .bind(user_id)
        .fetch_one(&self.core.db)
        .await?;
        self.push_space(&space).await;
        Ok(space)
    }
}
