//! Hangouts (たまり場): voice rooms that belong to a group of friends.
//!
//! A hangout has an owner, members and open invitations
//! (`hangout_members`), and a voice room that is simply open whenever
//! somebody is inside (`hangout_presence`). Everyone inside may talk.
//! Changes happen under a lock on the hangout's row, like rooms; members
//! hear of them on their own realtime subject, since a hangout has no
//! public topic.
//!
//! Nobody's trace is erased at once: someone who leaves, is removed or
//! declines stays on record as a former member for a while, and a hangout
//! its owner closes is only marked as deleted. To everyone it reads as
//! gone; the people who were part of it can still report it.
use tokuly_core::{events, outbound, prelude::*};
use tokuly_proto::{common::UserSummary, spaces as pb};
use tokuly_social::{InteractionKind, graph};

use crate::{
    Spaces,
    audio::HANGOUT_ROLE,
    events::{HANGOUT_JOINED, HangoutJoined},
    invites::Invited,
    limits::Limit,
    model::{HANGOUT, HangoutMemberRow, HangoutRow, Venue, member},
    notices,
    rooms::{Tx, ensure_nowhere_else, lock_presence},
    rules,
};

pub(crate) fn hangout_not_found() -> AppError {
    AppError::not_found("HANGOUT_NOT_FOUND", "hangout not found")
}

fn not_owner() -> AppError {
    AppError::denied("NOT_OWNER", "only the owner of the hangout can do this")
}

fn hangout_full() -> AppError {
    AppError::precondition("HANGOUT_FULL", "this hangout has no room for more people")
}

fn too_many_hangouts() -> AppError {
    AppError::precondition("TOO_MANY_HANGOUTS", "that is as many hangouts as one person can have")
}

fn target_not_a_member() -> AppError {
    AppError::precondition("TARGET_NOT_A_MEMBER", "this person is not part of the hangout")
}

const MEMBER: &str = "m.state, m.invited_by";

/// Locks a hangout for the rest of the transaction. One that was closed
/// is not found.
pub(crate) async fn lock_hangout(tx: &mut Tx<'_>, hangout_id: Uuid) -> AppResult<HangoutRow> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {HANGOUT} FROM hangouts h WHERE h.id = $1 AND h.deleted_at IS NULL FOR UPDATE"
    )))
    .bind(hangout_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(hangout_not_found)
}

/// Recomputes a hangout's counters; `active` also notes that somebody was
/// inside just now. Returns the hangout as it is now.
pub(crate) async fn recount_hangout(tx: &mut Tx<'_>, hangout_id: Uuid, active: bool) -> AppResult<HangoutRow> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "UPDATE hangouts h SET
             member_count = (SELECT count(*) FROM hangout_members m WHERE m.hangout_id = h.id AND m.state = $2)::int,
             inside_count = (SELECT count(*) FROM hangout_presence p WHERE p.hangout_id = h.id)::int,
             last_active_at = CASE WHEN $3 THEN now() ELSE h.last_active_at END
         WHERE h.id = $1
         RETURNING {HANGOUT}"
    )))
    .bind(hangout_id)
    .bind(member::MEMBER)
    .bind(active)
    .fetch_one(&mut **tx)
    .await?)
}

/// Someone's place in a hangout, if they are a member or invited (a former
/// member is not part of it).
async fn member_in_tx(tx: &mut Tx<'_>, hangout_id: Uuid, user_id: Uuid) -> AppResult<Option<HangoutMemberRow>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {MEMBER} FROM hangout_members m WHERE m.hangout_id = $1 AND m.user_id = $2 AND m.state <> $3"
    )))
    .bind(hangout_id)
    .bind(user_id)
    .bind(member::FORMER)
    .fetch_optional(&mut **tx)
    .await?)
}

/// Takes someone out of a hangout for good: their membership or
/// invitation (which stays on record as former) and their place in the
/// voice room. Returns whether they were in the voice room.
async fn expel(tx: &mut Tx<'_>, hangout_id: Uuid, user_id: Uuid) -> AppResult<bool> {
    sqlx::query(
        "UPDATE hangout_members SET state = $3, left_at = now(), muted = false
         WHERE hangout_id = $1 AND user_id = $2 AND state <> $3",
    )
    .bind(hangout_id)
    .bind(user_id)
    .bind(member::FORMER)
    .execute(&mut **tx)
    .await?;
    let inside = sqlx::query("DELETE FROM hangout_presence WHERE hangout_id = $1 AND user_id = $2")
        .bind(hangout_id)
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    Ok(inside.rows_affected() > 0)
}

/// Serializes one person's changes to how many hangouts they own or belong
/// to, so the limits hold.
pub(crate) async fn lock_person(tx: &mut Tx<'_>, user_id: Uuid) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("spaces:hangouts:{}", user_id.simple()))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

impl Spaces {
    /// The hangout and the caller's place in it, for members and invitees;
    /// `HANGOUT_NOT_FOUND` for everyone else.
    pub(crate) async fn my_hangout(
        &self,
        user_id: Uuid,
        hangout_id: Uuid,
    ) -> AppResult<(HangoutRow, HangoutMemberRow)> {
        let db = &self.core.db;
        let me: HangoutMemberRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {MEMBER} FROM hangout_members m WHERE m.hangout_id = $1 AND m.user_id = $2 AND m.state <> $3"
        )))
        .bind(hangout_id)
        .bind(user_id)
        .bind(member::FORMER)
        .fetch_optional(db)
        .await?
        .ok_or_else(hangout_not_found)?;
        let hangout: HangoutRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {HANGOUT} FROM hangouts h WHERE h.id = $1 AND h.deleted_at IS NULL"
        )))
        .bind(hangout_id)
        .fetch_optional(db)
        .await?
        .ok_or_else(hangout_not_found)?;
        Ok((hangout, me))
    }

    /// Everyone who belongs to a hangout; with `invitees` also the people
    /// who were asked and have not answered.
    pub(crate) async fn hangout_people(&self, hangout_id: Uuid, invitees: bool) -> AppResult<Vec<Uuid>> {
        Ok(sqlx::query_scalar(
            "SELECT user_id FROM hangout_members WHERE hangout_id = $1 AND (state = $2 OR (state = $3 AND $4))",
        )
        .bind(hangout_id)
        .bind(member::MEMBER)
        .bind(member::INVITED)
        .bind(invitees)
        .fetch_all(&self.core.db)
        .await?)
    }

    /// Tells members and invitees how the hangout looks now.
    pub(crate) async fn push_hangout(&self, hangout_id: Uuid) {
        let pushed: AppResult<()> = async {
            let Some((_, proto)) = self.hangout_snapshot(hangout_id).await? else { return Ok(()) };
            let people = self.hangout_people(hangout_id, true).await?;
            self.push_users(people, pb::HangoutUpdated { hangout: proto.into(), ..Default::default() }).await;
            Ok(())
        }
        .await;
        if let Err(e) = pushed {
            tracing::warn!(%hangout_id, error = %e, "hangout push failed");
        }
    }

    /// Tells the members that someone entered or left the voice room.
    async fn push_presence(&self, hangout: &HangoutRow, user_id: Uuid, inside: bool) {
        let pushed: AppResult<()> = async {
            let presence = pb::HangoutPresence {
                hangout_id: hangout.id.to_string(),
                user: tokuly_identity::summary(&self.core, user_id).await?.into(),
                inside,
                inside_count: hangout.inside_count,
                ..Default::default()
            };
            let mut members = self.hangout_people(hangout.id, false).await?;
            if !members.contains(&user_id) {
                // Someone who was just removed still learns that they are out.
                members.push(user_id);
            }
            self.push_users(members, presence).await;
            Ok(())
        }
        .await;
        if let Err(e) = pushed {
            tracing::warn!(hangout_id = %hangout.id, error = %e, "hangout presence push failed");
        }
    }

    /// Stores invitations to a hangout, in the caller's transaction, and
    /// requests a notification for those the inviter may still ring (see
    /// [`Spaces::may_announce_invitation`]). Someone who is part of the
    /// hangout already is skipped; a former member is invited afresh.
    async fn invite_to_hangout_in_tx(
        &self,
        tx: &mut Tx<'_>,
        hangout: &HangoutRow,
        inviter_id: Uuid,
        inviter: &UserSummary,
        invitees: &[Uuid],
    ) -> AppResult<Invited> {
        if invitees.is_empty() {
            return Ok(Invited::default());
        }
        let invited: Vec<Uuid> = sqlx::query_scalar(
            "INSERT INTO hangout_members AS m (hangout_id, user_id, state, invited_by)
             SELECT $1, invitee, $3, $4 FROM unnest($2::uuid[]) AS invitee
             ON CONFLICT (hangout_id, user_id) DO UPDATE
                 SET state = EXCLUDED.state, invited_by = EXCLUDED.invited_by, muted = false, joined_at = NULL,
                     left_at = NULL, created_at = now()
                 WHERE m.state = $5
             RETURNING user_id",
        )
        .bind(hangout.id)
        .bind(invitees)
        .bind(member::INVITED)
        .bind(inviter_id)
        .bind(member::FORMER)
        .fetch_all(&mut **tx)
        .await?;
        let people: i64 =
            sqlx::query_scalar("SELECT count(*) FROM hangout_members WHERE hangout_id = $1 AND state <> $2")
                .bind(hangout.id)
                .bind(member::FORMER)
                .fetch_one(&mut **tx)
                .await?;
        if people > self.cfg.max_hangout_members {
            return Err(hangout_full());
        }
        let mut announced = Vec::with_capacity(invited.len());
        for invitee in &invited {
            if self.may_announce_invitation(inviter_id, *invitee).await? {
                outbound::notify(&mut **tx, &notices::hangout_invitation(*invitee, inviter, inviter_id, hangout))
                    .await?;
                announced.push(*invitee);
            }
        }
        Ok(Invited { all: invited, announced })
    }

    /// Tells newly invited people over realtime.
    async fn announce_hangout_invitations(&self, hangout_id: Uuid, inviter: &UserSummary, invited: &[Uuid]) {
        if invited.is_empty() {
            return;
        }
        let pushed: AppResult<()> = async {
            let Some((_, proto)) = self.hangout_snapshot(hangout_id).await? else { return Ok(()) };
            let invitation = pb::HangoutInvitation {
                hangout: proto.into(),
                invited_by: inviter.clone().into(),
                ..Default::default()
            };
            self.push_users(invited.iter().copied(), invitation).await;
            Ok(())
        }
        .await;
        if let Err(e) = pushed {
            tracing::warn!(%hangout_id, error = %e, "hangout invitation push failed");
        }
    }

    // ───────────────────────────── the group ─────────────────────────────

    /// Creates a hangout owned by `user_id` and invites `invitee_ids`.
    pub(crate) async fn create_hangout(
        &self,
        user_id: Uuid,
        request: pb::CreateHangoutRequest,
    ) -> AppResult<HangoutRow> {
        let core = &self.core;
        let name = rules::hangout_name(&request.name)?;
        let emoji = match request.emoji.trim() {
            "" => rules::DEFAULT_HANGOUT_EMOJI.to_owned(),
            emoji => rules::emoji(emoji)?,
        };
        let invitees = self.invitable(user_id, user_id, &request.invitee_ids).await?;
        self.limit(Limit::CreateHangout, user_id).await?;
        // Everything the transaction needs from elsewhere is fetched first:
        // while it holds its locks it runs on its own connection only.
        let owner = tokuly_identity::summary(core, user_id).await?;

        let hangout_id = new_id();
        let mut tx = core.db.begin().await?;
        lock_person(&mut tx, user_id).await?;
        let (owned, belonging): (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM hangouts WHERE owner_id = $1 AND deleted_at IS NULL),
                    (SELECT count(*) FROM hangout_members WHERE user_id = $1 AND state = $2)",
        )
        .bind(user_id)
        .bind(member::MEMBER)
        .fetch_one(&mut *tx)
        .await?;
        if owned >= self.cfg.max_hangouts || belonging >= self.cfg.max_hangout_memberships {
            return Err(too_many_hangouts());
        }
        let hangout: HangoutRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO hangouts AS h (id, owner_id, name, emoji, members_can_invite, room)
             VALUES ($1, $2, $3, $4, $5, $6)
             RETURNING {HANGOUT}"
        )))
        .bind(hangout_id)
        .bind(user_id)
        .bind(&name)
        .bind(&emoji)
        .bind(request.members_can_invite)
        .bind(Venue::Hangout(hangout_id).room())
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO hangout_members (hangout_id, user_id, state, joined_at) VALUES ($1, $2, $3, now())")
            .bind(hangout_id)
            .bind(user_id)
            .bind(member::MEMBER)
            .execute(&mut *tx)
            .await?;
        let invited = self.invite_to_hangout_in_tx(&mut tx, &hangout, user_id, &owner, &invitees).await?;
        let hangout = recount_hangout(&mut tx, hangout_id, false).await?;
        tx.commit().await?;
        self.announce_hangout_invitations(hangout_id, &owner, &invited.announced).await;
        Ok(hangout)
    }

    /// Renames a hangout, changes its emoji or who may invite.
    pub(crate) async fn update_hangout(
        &self,
        user_id: Uuid,
        request: pb::UpdateHangoutRequest,
    ) -> AppResult<HangoutRow> {
        let hangout_id = parse_id(&request.hangout_id)?;
        let (hangout, _) = self.my_hangout(user_id, hangout_id).await?;
        if hangout.owner_id != user_id {
            return Err(not_owner());
        }
        self.limit(Limit::ManageHangout, user_id).await?;
        let name = request.name.as_deref().map(rules::hangout_name).transpose()?;
        let emoji = request.emoji.as_deref().map(rules::emoji).transpose()?;
        let hangout: HangoutRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE hangouts h SET name = COALESCE($3, h.name), emoji = COALESCE($4, h.emoji),
                 members_can_invite = COALESCE($5, h.members_can_invite)
             WHERE h.id = $1 AND h.owner_id = $2 AND h.deleted_at IS NULL
             RETURNING {HANGOUT}"
        )))
        .bind(hangout_id)
        .bind(user_id)
        .bind(&name)
        .bind(&emoji)
        .bind(request.members_can_invite)
        .fetch_optional(&self.core.db)
        .await?
        .ok_or_else(not_owner)?;
        self.push_hangout(hangout_id).await;
        Ok(hangout)
    }

    /// Closes a hangout for good.
    pub(crate) async fn delete_hangout(&self, user_id: Uuid, hangout_id: Uuid) -> AppResult<()> {
        let (hangout, _) = self.my_hangout(user_id, hangout_id).await?;
        if hangout.owner_id != user_id {
            return Err(not_owner());
        }
        self.dissolve_hangout(hangout_id).await
    }

    /// Closes a hangout: it is marked as deleted, everyone who was part of
    /// it becomes a former member and is told, and its voice room is
    /// closed. The record stays until the sweep purges it, so that it can
    /// still be reported. A hangout that is already gone is done.
    pub(crate) async fn dissolve_hangout(&self, hangout_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        if lock_hangout(&mut tx, hangout_id).await.is_err() {
            return Ok(());
        }
        let people: Vec<Uuid> = sqlx::query_scalar(
            "UPDATE hangout_members SET state = $2, left_at = now(), muted = false
             WHERE hangout_id = $1 AND state <> $2
             RETURNING user_id",
        )
        .bind(hangout_id)
        .bind(member::FORMER)
        .fetch_all(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM hangout_presence WHERE hangout_id = $1").bind(hangout_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE hangouts SET deleted_at = now(), inside_count = 0, member_count = 0 WHERE id = $1")
            .bind(hangout_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.push_users(people, pb::HangoutRemoved { hangout_id: hangout_id.to_string(), ..Default::default() }).await;
        self.close_room(Venue::Hangout(hangout_id)).await;
        Ok(())
    }

    /// The caller's hangouts — open ones first, then the most recently
    /// used — and the invitations waiting for their answer.
    pub(crate) async fn list_hangouts(&self, user_id: Uuid) -> AppResult<(Vec<pb::Hangout>, Vec<pb::Hangout>)> {
        let listed = |state: i16| async move {
            let rows: Vec<HangoutRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {HANGOUT} FROM hangouts h
                 JOIN hangout_members m ON m.hangout_id = h.id AND m.user_id = $1 AND m.state = $2
                 WHERE h.deleted_at IS NULL
                 ORDER BY (h.inside_count > 0) DESC, COALESCE(h.last_active_at, h.created_at) DESC, h.id DESC"
            )))
            .bind(user_id)
            .bind(state)
            .fetch_all(&self.core.db)
            .await?;
            self.render_hangouts(&rows, Some(user_id)).await
        };
        Ok((listed(member::MEMBER).await?, listed(member::INVITED).await?))
    }

    /// Invites friends of the caller. Returns the hangout and how many new
    /// invitations went out.
    pub(crate) async fn invite_to_hangout(
        &self,
        user_id: Uuid,
        hangout_id: Uuid,
        raw_ids: &[String],
    ) -> AppResult<(HangoutRow, i32)> {
        let core = &self.core;
        let (hangout, me) = self.my_hangout(user_id, hangout_id).await?;
        let may_invite = |hangout: &HangoutRow, me: &HangoutMemberRow| {
            me.is_member() && (hangout.owner_id == user_id || hangout.members_can_invite)
        };
        ensure!(may_invite(&hangout, &me), denied, "INVITES_NOT_ALLOWED", "you cannot invite people to this hangout");
        let invitees = self.invitable(user_id, hangout.owner_id, raw_ids).await?;
        if invitees.is_empty() {
            return Ok((hangout, 0));
        }
        self.limit(Limit::InviteToHangout, user_id).await?;
        // Fetched before the hangout is locked (see `create_hangout`).
        let inviter = tokuly_identity::summary(core, user_id).await?;

        let mut tx = core.db.begin().await?;
        let locked = lock_hangout(&mut tx, hangout_id).await?;
        let me = member_in_tx(&mut tx, hangout_id, user_id).await?.ok_or_else(hangout_not_found)?;
        ensure!(may_invite(&locked, &me), denied, "INVITES_NOT_ALLOWED", "you cannot invite people to this hangout");
        let invited = self.invite_to_hangout_in_tx(&mut tx, &locked, user_id, &inviter, &invitees).await?;
        let hangout = recount_hangout(&mut tx, hangout_id, false).await?;
        tx.commit().await?;
        if !invited.all.is_empty() {
            self.announce_hangout_invitations(hangout_id, &inviter, &invited.announced).await;
            self.push_hangout(hangout_id).await;
        }
        Ok((hangout, invited.all.len() as i32))
    }

    /// Accepts or declines an invitation. Returns the hangout when it was
    /// accepted.
    pub(crate) async fn respond_to_hangout_invite(
        &self,
        user_id: Uuid,
        hangout_id: Uuid,
        accept: bool,
    ) -> AppResult<Option<HangoutRow>> {
        let mut tx = self.core.db.begin().await?;
        if accept {
            lock_person(&mut tx, user_id).await?;
        }
        let hangout = lock_hangout(&mut tx, hangout_id).await?;
        let me = member_in_tx(&mut tx, hangout_id, user_id).await?.ok_or_else(hangout_not_found)?;
        ensure!(me.state == member::INVITED, precondition, "NO_INVITATION", "there is no invitation to answer");
        if !accept {
            expel(&mut tx, hangout_id, user_id).await?;
            recount_hangout(&mut tx, hangout_id, false).await?;
            tx.commit().await?;
            self.push_user(user_id, pb::HangoutRemoved { hangout_id: hangout_id.to_string(), ..Default::default() })
                .await;
            self.push_hangout(hangout_id).await;
            return Ok(None);
        }
        let belonging: i64 =
            sqlx::query_scalar("SELECT count(*) FROM hangout_members WHERE user_id = $1 AND state = $2")
                .bind(user_id)
                .bind(member::MEMBER)
                .fetch_one(&mut *tx)
                .await?;
        if belonging >= self.cfg.max_hangout_memberships {
            return Err(too_many_hangouts());
        }
        if i64::from(hangout.member_count) >= self.cfg.max_hangout_members {
            return Err(hangout_full());
        }
        sqlx::query("UPDATE hangout_members SET state = $3, joined_at = now() WHERE hangout_id = $1 AND user_id = $2")
            .bind(hangout_id)
            .bind(user_id)
            .bind(member::MEMBER)
            .execute(&mut *tx)
            .await?;
        let hangout = recount_hangout(&mut tx, hangout_id, false).await?;
        tx.commit().await?;
        self.push_hangout(hangout_id).await;
        Ok(Some(hangout))
    }

    /// What follows someone being taken out of a hangout, once that is
    /// committed.
    async fn after_expel(&self, hangout: &HangoutRow, user_id: Uuid, was_inside: bool) {
        self.push_user(user_id, pb::HangoutRemoved { hangout_id: hangout.id.to_string(), ..Default::default() }).await;
        if was_inside {
            self.push_presence(hangout, user_id, false).await;
            self.kick(Venue::Hangout(hangout.id), user_id).await;
        }
        self.push_hangout(hangout.id).await;
    }

    /// Removes a member, or takes an invitation back.
    pub(crate) async fn remove_hangout_member(
        &self,
        actor: Uuid,
        hangout_id: Uuid,
        target: Uuid,
    ) -> AppResult<HangoutRow> {
        self.limit(Limit::ManageHangout, actor).await?;
        let mut tx = self.core.db.begin().await?;
        let hangout = lock_hangout(&mut tx, hangout_id).await?;
        member_in_tx(&mut tx, hangout_id, actor).await?.ok_or_else(hangout_not_found)?;
        ensure!(
            target != hangout.owner_id && target != actor,
            invalid,
            "INVALID_USER",
            "this cannot be done to that person"
        );
        let other = member_in_tx(&mut tx, hangout_id, target).await?.ok_or_else(target_not_a_member)?;
        let own_invitation = other.state == member::INVITED && other.invited_by == Some(actor);
        if hangout.owner_id != actor && !own_invitation {
            return Err(not_owner());
        }
        let was_inside = expel(&mut tx, hangout_id, target).await?;
        let hangout = recount_hangout(&mut tx, hangout_id, false).await?;
        tx.commit().await?;
        self.after_expel(&hangout, target, was_inside).await;
        Ok(hangout)
    }

    /// Takes `user_id` out of a hangout because of something that happened
    /// elsewhere (a block, a closed account). Does nothing for the owner
    /// or for someone who is not part of it.
    pub(crate) async fn drop_from_hangout(&self, hangout_id: Uuid, user_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let Ok(hangout) = lock_hangout(&mut tx, hangout_id).await else { return Ok(()) };
        if hangout.owner_id == user_id || member_in_tx(&mut tx, hangout_id, user_id).await?.is_none() {
            return Ok(());
        }
        let was_inside = expel(&mut tx, hangout_id, user_id).await?;
        let hangout = recount_hangout(&mut tx, hangout_id, false).await?;
        tx.commit().await?;
        self.after_expel(&hangout, user_id, was_inside).await;
        Ok(())
    }

    /// Stops being a member.
    pub(crate) async fn quit_hangout(&self, user_id: Uuid, hangout_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let hangout = lock_hangout(&mut tx, hangout_id).await?;
        member_in_tx(&mut tx, hangout_id, user_id).await?.ok_or_else(hangout_not_found)?;
        ensure!(
            hangout.owner_id != user_id,
            precondition,
            "OWNER_CANNOT_QUIT",
            "hand the hangout over or delete it instead"
        );
        let was_inside = expel(&mut tx, hangout_id, user_id).await?;
        let hangout = recount_hangout(&mut tx, hangout_id, false).await?;
        tx.commit().await?;
        self.after_expel(&hangout, user_id, was_inside).await;
        Ok(())
    }

    /// Makes another member the owner.
    pub(crate) async fn transfer_hangout(
        &self,
        user_id: Uuid,
        hangout_id: Uuid,
        target: Uuid,
    ) -> AppResult<HangoutRow> {
        self.limit(Limit::ManageHangout, user_id).await?;
        let mut tx = self.core.db.begin().await?;
        lock_person(&mut tx, target).await?;
        let hangout = lock_hangout(&mut tx, hangout_id).await?;
        member_in_tx(&mut tx, hangout_id, user_id).await?.ok_or_else(hangout_not_found)?;
        if hangout.owner_id != user_id {
            return Err(not_owner());
        }
        ensure!(target != user_id, invalid, "INVALID_USER", "the hangout is yours already");
        member_in_tx(&mut tx, hangout_id, target)
            .await?
            .filter(HangoutMemberRow::is_member)
            .ok_or_else(target_not_a_member)?;
        let owned: i64 = sqlx::query_scalar("SELECT count(*) FROM hangouts WHERE owner_id = $1 AND deleted_at IS NULL")
            .bind(target)
            .fetch_one(&mut *tx)
            .await?;
        if owned >= self.cfg.max_hangouts {
            return Err(too_many_hangouts());
        }
        let hangout: HangoutRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE hangouts h SET owner_id = $2 WHERE h.id = $1 RETURNING {HANGOUT}"
        )))
        .bind(hangout_id)
        .bind(target)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        self.push_hangout(hangout_id).await;
        Ok(hangout)
    }

    /// Silences or restores "someone is in the hangout" notifications.
    pub(crate) async fn set_hangout_muted(
        &self,
        user_id: Uuid,
        hangout_id: Uuid,
        muted: bool,
    ) -> AppResult<HangoutRow> {
        let (hangout, _) = self.my_hangout(user_id, hangout_id).await?;
        sqlx::query("UPDATE hangout_members SET muted = $3 WHERE hangout_id = $1 AND user_id = $2")
            .bind(hangout_id)
            .bind(user_id)
            .bind(muted)
            .execute(&self.core.db)
            .await?;
        Ok(hangout)
    }

    // ───────────────────────────── the voice room ─────────────────────────────

    /// Enters a hangout's voice room. Whoever is inside already gets fresh
    /// credentials.
    pub(crate) async fn join_hangout(
        &self,
        user_id: Uuid,
        hangout_id: Uuid,
    ) -> AppResult<(HangoutRow, pb::AudioCredentials)> {
        let core = &self.core;
        let (_, me) = self.my_hangout(user_id, hangout_id).await?;
        ensure!(me.is_member(), precondition, "NOT_A_MEMBER", "accept the invitation first");
        self.require_audio()?;
        self.limit(Limit::Enter, user_id).await?;
        // Fetched before the hangout is locked: while the transaction holds
        // its locks it runs on its own connection only.
        let who = tokuly_identity::summary(core, user_id).await?;
        let friends = graph::friend_ids(core, user_id).await?;
        let venue = Venue::Hangout(hangout_id);
        self.open_room(venue).await?;

        let enter = || async {
            let mut tx = core.db.begin().await?;
            lock_presence(&mut tx, user_id).await?;
            lock_hangout(&mut tx, hangout_id).await?;
            member_in_tx(&mut tx, hangout_id, user_id)
                .await?
                .filter(HangoutMemberRow::is_member)
                .ok_or_else(hangout_not_found)?;
            // One room at a time.
            ensure_nowhere_else(&mut tx, user_id, venue).await?;
            // Coming in again (a reconnect, another device) is a sign of life.
            let entered: bool = sqlx::query_scalar(
                "INSERT INTO hangout_presence (hangout_id, user_id) VALUES ($1, $2)
                 ON CONFLICT (hangout_id, user_id) DO UPDATE SET seen_at = now(), missing_since = NULL
                 RETURNING (xmax = 0)",
            )
            .bind(hangout_id)
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
            let hangout = recount_hangout(&mut tx, hangout_id, true).await?;
            if entered {
                let first = hangout.inside_count == 1;
                let joined = HangoutJoined { hangout_id, user_id, first, inside_count: hangout.inside_count };
                events::emit(&mut *tx, HANGOUT_JOINED, Some(user_id), &joined).await?;
                let quiet_until = hangout
                    .last_announced_at
                    .map(|at| at + chrono::Duration::seconds(self.cfg.hangout_announce_seconds));
                if first && quiet_until.is_none_or(|until| until <= now()) {
                    self.announce_open_hangout(&mut tx, &hangout, user_id, &who, &friends).await?;
                }
            }
            tx.commit().await?;
            Ok((hangout, entered))
        };
        let (hangout, entered) = self.entering(user_id, venue, enter).await?;
        if entered {
            self.push_presence(&hangout, user_id, true).await;
        }
        let credentials = self.credentials(venue, user_id, &notices::name_of(&who), HANGOUT_ROLE)?;
        Ok((hangout, credentials))
    }

    /// Tells the members who are friends of the first person inside that
    /// the hangout is open — except those who silenced it. The notify
    /// domain applies per-friend mutes and quiet hours on top.
    async fn announce_open_hangout(
        &self,
        tx: &mut Tx<'_>,
        hangout: &HangoutRow,
        user_id: Uuid,
        who: &UserSummary,
        friends: &[Uuid],
    ) -> AppResult<()> {
        let recipients: Vec<Uuid> = sqlx::query_scalar(
            "SELECT user_id FROM hangout_members
             WHERE hangout_id = $1 AND state = $2 AND NOT muted AND user_id <> $3 AND user_id = ANY($4)",
        )
        .bind(hangout.id)
        .bind(member::MEMBER)
        .bind(user_id)
        .bind(friends)
        .fetch_all(&mut **tx)
        .await?;
        if recipients.is_empty() {
            return Ok(());
        }
        sqlx::query("UPDATE hangouts SET last_announced_at = now() WHERE id = $1")
            .bind(hangout.id)
            .execute(&mut **tx)
            .await?;
        for recipient in recipients {
            outbound::notify(&mut **tx, &notices::hangout_joined(recipient, who, user_id, hangout)).await?;
        }
        Ok(())
    }

    /// Leaves the voice room; the membership stays.
    pub(crate) async fn leave_hangout(&self, user_id: Uuid, hangout_id: Uuid) -> AppResult<()> {
        self.my_hangout(user_id, hangout_id).await?;
        self.remove_from_hangout_voice(hangout_id, &[user_id], true).await?;
        Ok(())
    }

    /// Takes people out of a hangout's voice room and tells the members.
    /// `disconnect` also removes them from LiveKit (not needed for people
    /// whose connection is gone already). Returns who actually was inside.
    pub(crate) async fn remove_from_hangout_voice(
        &self,
        hangout_id: Uuid,
        user_ids: &[Uuid],
        disconnect: bool,
    ) -> AppResult<Vec<Uuid>> {
        let mut tx = self.core.db.begin().await?;
        match lock_hangout(&mut tx, hangout_id).await {
            Ok(_) => {}
            // Closed in the meantime: nobody is inside any more.
            Err(e) if e.is_not_found() => return Ok(Vec::new()),
            Err(e) => return Err(e),
        }
        let Some(vacated) = vacate(&mut tx, hangout_id, user_ids).await? else { return Ok(Vec::new()) };
        tx.commit().await?;
        self.after_vacate(&vacated, disconnect).await;
        Ok(vacated.gone.into_iter().map(|(user_id, _)| user_id).collect())
    }

    /// What follows people leaving a hangout's voice room, once that is
    /// committed: the members see them go, LiveKit disconnects them
    /// (`disconnect`), and the time they spent with the others counts for
    /// the friendships.
    pub(crate) async fn after_vacate(&self, vacated: &Vacated, disconnect: bool) {
        let venue = Venue::Hangout(vacated.hangout.id);
        for (user_id, _) in &vacated.gone {
            self.push_presence(&vacated.hangout, *user_id, false).await;
            if disconnect {
                self.kick(venue, *user_id).await;
            }
        }
        self.record_time_together(&vacated.gone, &vacated.staying).await;
    }

    /// Reports the time people spent in a hangout together to the
    /// best-friend ranking, as time on a call. Each pair is counted once,
    /// when the first of the two leaves.
    async fn record_time_together(&self, gone: &[(Uuid, Time)], staying: &[(Uuid, Time)]) {
        let now = now();
        let minutes = |a: Time, b: Time| ((now - a.max(b)).num_seconds() / 60).clamp(0, i64::from(i32::MAX)) as i32;
        for (index, (leaver, since)) in gone.iter().enumerate() {
            for (other, other_since) in staying.iter().chain(&gone[index + 1..]) {
                let together = minutes(*since, *other_since);
                if together == 0 {
                    continue;
                }
                // Ignored unless the two are friends.
                if let Err(e) =
                    graph::record_interaction(&self.core, *leaver, *other, InteractionKind::Call, together).await
                {
                    tracing::warn!(error = %e, "time in a hangout could not be reported");
                }
            }
        }
    }
}

/// People who just left a hangout's voice room, and what is left of it.
#[derive(Debug, Clone)]
pub(crate) struct Vacated {
    /// The hangout after they left.
    pub hangout: HangoutRow,
    /// Who left, with when they had entered.
    pub gone: Vec<(Uuid, Time)>,
    /// Who is still inside, with when they entered.
    pub staying: Vec<(Uuid, Time)>,
}

/// Takes people out of a hangout's voice room, inside the transaction that
/// holds the hangout's lock. `None` when none of them was inside. Follow
/// the commit with [`Spaces::after_vacate`].
pub(crate) async fn vacate(tx: &mut Tx<'_>, hangout_id: Uuid, user_ids: &[Uuid]) -> AppResult<Option<Vacated>> {
    let gone: Vec<(Uuid, Time)> = sqlx::query_as(
        "DELETE FROM hangout_presence WHERE hangout_id = $1 AND user_id = ANY($2) RETURNING user_id, joined_at",
    )
    .bind(hangout_id)
    .bind(user_ids)
    .fetch_all(&mut **tx)
    .await?;
    if gone.is_empty() {
        return Ok(None);
    }
    let staying: Vec<(Uuid, Time)> =
        sqlx::query_as("SELECT user_id, joined_at FROM hangout_presence WHERE hangout_id = $1")
            .bind(hangout_id)
            .fetch_all(&mut **tx)
            .await?;
    let hangout = recount_hangout(tx, hangout_id, true).await?;
    Ok(Some(Vacated { hangout, gone, staying }))
}
