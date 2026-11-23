//! Reactions to what happens elsewhere: an account is erased or suspended,
//! one person blocks another, moderation takes a room down — and this
//! domain's own "a room went live", which fans the news out.
//!
//! Events are delivered at least once and not always in order. Every
//! handler gives the same result when its event arrives twice, and the
//! ones about a state that can be undone (a suspension, a block) look at
//! how things are now rather than taking the event's word for it.
use std::time::Duration;

use tokuly_core::{events::Event, outbound, prelude::*};
use tokuly_identity::users;
use tokuly_proto::common::SubjectType;
use tokuly_social::graph;

use crate::{
    Spaces,
    events::{
        CONTENT_REMOVED, ContentRemoved, FRIEND_REMOVED, FriendRemoved, ROOM_STARTED, RoomStarted, USER_BANNED,
        USER_BLOCKED, USER_DELETED, UserBlocked, UserGone,
    },
    hangouts::lock_hangout,
    model::{audience, end, leave, member, role, state},
    notices,
    rooms::lock_space,
};

/// Notifications requested per transaction when a room starts.
const ANNOUNCE_BATCH: i64 = 500;

impl Spaces {
    /// Handles one of the events the domain consumes (see
    /// [`crate::workers`] for the list).
    pub async fn handle_event(&self, event: &Event) -> AppResult<()> {
        match event.topic.as_str() {
            USER_DELETED => self.on_user_deleted(event.data::<UserGone>()?.user_id).await,
            USER_BANNED => {
                let user_id = event.data::<UserGone>()?.user_id;
                // News of a suspension that was lifted since is no news.
                if users::filter_active(&self.core, &[user_id]).await?.is_empty() {
                    self.close_account_rooms(user_id).await?;
                }
                Ok(())
            }
            USER_BLOCKED => {
                let block: UserBlocked = event.data()?;
                // Likewise for a block that was lifted since.
                if graph::is_blocked_either(&self.core, block.user_id, block.blocked_id).await? {
                    self.on_user_blocked(block.user_id, block.blocked_id).await?;
                }
                Ok(())
            }
            FRIEND_REMOVED => {
                // A friends room of either is no longer the other's to follow
                // (unless they are inside or invited: the guard decides).
                let removed: FriendRemoved = event.data()?;
                self.core.rt.revoke_prefix(removed.user_id, "space").await;
                self.core.rt.revoke_prefix(removed.friend_id, "space").await;
                Ok(())
            }
            CONTENT_REMOVED => self.on_content_removed(event.data()?).await,
            ROOM_STARTED => self.announce_start(event.data()?).await,
            _ => Ok(()),
        }
    }

    // ───────────────────────────── a room went live ─────────────────────────────

    /// `spaces.room.started`: tells the people who are waiting for it.
    /// Everyone who asked to be reminded gets `spaces.reminder`; for a
    /// friends room the host's friends get `spaces.started`. Nobody else
    /// hears of a public room.
    ///
    /// Reminders are deleted as they are sent and the friends are only
    /// told once per room, so a second delivery of the event sends nothing.
    /// A host who opens one friends room after another rings their friends'
    /// phones for the first one only (`TOKULY_SPACES_STARTED_ANNOUNCE_SECONDS`).
    pub(crate) async fn announce_start(&self, started: RoomStarted) -> AppResult<()> {
        let core = &self.core;
        let Some(space) = self.find_space(started.space_id).await? else { return Ok(()) };
        if !space.is_live() {
            return Ok(());
        }
        let host = tokuly_identity::summary(core, space.host_id).await?;
        let hidden = graph::blocked_ids_either(core, space.host_id).await?;
        let friends = graph::friend_ids(core, space.host_id).await?;

        let mut reminded: Vec<Uuid> = Vec::new();
        loop {
            let mut tx = core.db.begin().await?;
            let batch: Vec<Uuid> = sqlx::query_scalar(
                "DELETE FROM space_reminders
                 WHERE space_id = $1
                   AND user_id IN (SELECT user_id FROM space_reminders WHERE space_id = $1 ORDER BY user_id LIMIT $2)
                 RETURNING user_id",
            )
            .bind(space.id)
            .bind(ANNOUNCE_BATCH)
            .fetch_all(&mut *tx)
            .await?;
            if batch.is_empty() {
                break;
            }
            // A reminder set while the room could still be seen does not
            // outlive the access: friendship, invitation and bans decide now.
            let allowed: Vec<Uuid> = sqlx::query_scalar(
                "SELECT candidate FROM unnest($2::uuid[]) AS candidate
                 WHERE candidate <> $3 AND candidate <> ALL($4)
                   AND NOT EXISTS (SELECT 1 FROM space_bans b WHERE b.space_id = $1 AND b.user_id = candidate)
                   AND ($5 = $6 OR ($5 = $7 AND candidate = ANY($8))
                        OR EXISTS (SELECT 1 FROM space_invites i
                                   WHERE i.space_id = $1 AND i.user_id = candidate AND i.revoked_at IS NULL))",
            )
            .bind(space.id)
            .bind(&batch)
            .bind(space.host_id)
            .bind(&hidden)
            .bind(space.audience)
            .bind(audience::PUBLIC)
            .bind(audience::FRIENDS)
            .bind(&friends)
            .fetch_all(&mut *tx)
            .await?;
            for recipient in &allowed {
                outbound::notify(&mut *tx, &notices::reminder(*recipient, &host, &space)).await?;
            }
            tx.commit().await?;
            reminded.extend(batch);
        }

        if space.audience != audience::FRIENDS {
            return Ok(());
        }
        let mut tx = core.db.begin().await?;
        // Claims the announcement, so the friends are told once.
        let claimed = sqlx::query(
            "UPDATE spaces SET attrs = attrs || '{\"announced\": true}'::jsonb
             WHERE id = $1 AND state = $2 AND NOT (attrs ? 'announced')",
        )
        .bind(space.id)
        .bind(state::LIVE)
        .execute(&mut *tx)
        .await?;
        if claimed.rows_affected() == 0 {
            return Ok(());
        }
        let cooldown = Duration::from_secs(self.cfg.started_announce_seconds);
        if !cooldown.is_zero()
            && !core.cache.set_nx(&format!("spaces:announced:{}", space.host_id), "1", cooldown).await?
        {
            // Their friends heard from this host a moment ago: this room
            // goes live quietly (the claim stays, so it stays quiet).
            tx.commit().await?;
            return Ok(());
        }
        let banned: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM space_bans WHERE space_id = $1")
            .bind(space.id)
            .fetch_all(&mut *tx)
            .await?;
        for friend in friends.iter().filter(|friend| !reminded.contains(friend) && !banned.contains(friend)) {
            outbound::notify(&mut *tx, &notices::started(*friend, &host, &space)).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    // ───────────────────────────── accounts ─────────────────────────────

    /// `identity.user.banned`, and the first half of an erased account:
    /// the person leaves whatever they are inside, the room they host ends
    /// and their upcoming rooms are called off.
    pub(crate) async fn close_account_rooms(&self, user_id: Uuid) -> AppResult<()> {
        self.leave_everything(user_id, None).await?;
        let hosted: Vec<(Uuid, i16)> =
            sqlx::query_as("SELECT id, state FROM spaces WHERE host_id = $1 AND state IN ($2, $3)")
                .bind(user_id)
                .bind(state::LIVE)
                .bind(state::SCHEDULED)
                .fetch_all(&self.core.db)
                .await?;
        for (space_id, _) in hosted {
            let mut tx = self.core.db.begin().await?;
            let Ok(space) = lock_space(&mut tx, space_id).await else { continue };
            if space.host_id != user_id {
                continue;
            }
            if space.is_live() {
                let finished = self.finish(&mut tx, &space, end::ACCOUNT_CLOSED).await?;
                tx.commit().await?;
                self.after_finish(&finished).await;
            } else if space.is_scheduled() {
                let cancelled = self.call_off(&mut tx, &space, end::ACCOUNT_CLOSED).await?;
                tx.commit().await?;
                self.push_space(&cancelled).await;
            }
        }
        Ok(())
    }

    /// `identity.user.deleted`: erases what the account left in this
    /// domain. Its rooms go (live ones end first), with everything on
    /// them; its places in other people's rooms, its invitations,
    /// reminders and bans go. A hangout it owned passes to the member who
    /// has belonged longest, or is closed when there is none; its other
    /// memberships end.
    pub(crate) async fn on_user_deleted(&self, user_id: Uuid) -> AppResult<()> {
        let db = &self.core.db;
        self.close_account_rooms(user_id).await?;

        let owned: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM hangouts WHERE owner_id = $1 AND deleted_at IS NULL")
            .bind(user_id)
            .fetch_all(db)
            .await?;
        for hangout_id in owned {
            let mut tx = db.begin().await?;
            let Ok(hangout) = lock_hangout(&mut tx, hangout_id).await else { continue };
            if hangout.owner_id != user_id {
                continue;
            }
            let heir: Option<Uuid> = sqlx::query_scalar(
                "SELECT user_id FROM hangout_members WHERE hangout_id = $1 AND state = $2 AND user_id <> $3
                 ORDER BY joined_at, user_id LIMIT 1",
            )
            .bind(hangout_id)
            .bind(member::MEMBER)
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
            match heir {
                Some(heir) => {
                    sqlx::query("UPDATE hangouts SET owner_id = $2 WHERE id = $1")
                        .bind(hangout_id)
                        .bind(heir)
                        .execute(&mut *tx)
                        .await?;
                    tx.commit().await?;
                    // The former owner is an ordinary member now and leaves below.
                }
                None => {
                    drop(tx);
                    self.dissolve_hangout(hangout_id).await?;
                }
            }
        }
        let memberships: Vec<Uuid> =
            sqlx::query_scalar("SELECT hangout_id FROM hangout_members WHERE user_id = $1 AND state <> $2")
                .bind(user_id)
                .bind(member::FORMER)
                .fetch_all(db)
                .await?;
        for hangout_id in memberships {
            self.drop_from_hangout(hangout_id, user_id).await?;
        }

        let mut tx = db.begin().await?;
        // Every room the account still hosts is over by now, and every
        // hangout it still owns is closed. Here "deleted" means gone: the
        // account's time to be reported ran out with identity's grace period.
        sqlx::query("DELETE FROM spaces WHERE host_id = $1").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM hangouts WHERE owner_id = $1").bind(user_id).execute(&mut *tx).await?;
        sqlx::query(
            "WITH gone AS (DELETE FROM space_reminders WHERE user_id = $1 RETURNING space_id)
             UPDATE spaces s SET reminder_count = GREATEST(s.reminder_count - 1, 0) WHERE s.id IN (SELECT space_id FROM gone)",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        for sql in [
            "DELETE FROM space_participants WHERE user_id = $1",
            "DELETE FROM space_bans WHERE user_id = $1",
            "DELETE FROM space_invites WHERE user_id = $1 OR invited_by = $1",
            "DELETE FROM hangout_members WHERE user_id = $1",
            "DELETE FROM hangout_presence WHERE user_id = $1",
            "UPDATE space_bans SET banned_by = NULL WHERE banned_by = $1",
            "UPDATE space_participants SET stage_invited_at = NULL, stage_invited_by = NULL WHERE stage_invited_by = $1",
            "UPDATE spaces SET pinned_by = NULL WHERE pinned_by = $1",
            "UPDATE hangout_members SET invited_by = NULL WHERE invited_by = $1",
        ] {
            sqlx::query(sql).bind(user_id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    // ───────────────────────────── blocks ─────────────────────────────

    /// `social.user.blocked`: the two cannot be in each other's rooms and
    /// hangouts any more.
    ///
    /// Blocks are silent, so nothing the *blocked* person can see may tell
    /// them. In what the blocker runs, the blocked person is put out the
    /// way a moderator would: removed from the live room, their invitation
    /// taken back, a co-host role kept for later dropped, out of the
    /// blocker's hangouts. In what the blocked person runs, the blocker
    /// simply walks away — leaves the room, quits the hangout or declines
    /// its invitation — and nothing else changes: no invitation or
    /// reminder disappears from the blocked person's lists. An open
    /// invitation between the two to somebody else's hangout goes, like a
    /// declined one. (Both keep their places in rooms and hangouts of
    /// third people; reminders are left alone and simply never fire.)
    pub(crate) async fn on_user_blocked(&self, blocker: Uuid, blocked: Uuid) -> AppResult<()> {
        let db = &self.core.db;
        let rooms_with = |host: Uuid, guest: Uuid| async move {
            sqlx::query_scalar::<_, Uuid>(
                "SELECT s.id FROM spaces s
                 JOIN space_participants p ON p.space_id = s.id AND p.user_id = $2 AND p.left_at IS NULL
                 WHERE s.host_id = $1 AND s.state = $3",
            )
            .bind(host)
            .bind(guest)
            .bind(state::LIVE)
            .fetch_all(db)
            .await
        };
        let hangouts_with = |owner: Uuid, other: Uuid| async move {
            sqlx::query_scalar::<_, Uuid>(
                "SELECT h.id FROM hangouts h
                 JOIN hangout_members m ON m.hangout_id = h.id AND m.user_id = $2 AND m.state <> $3
                 WHERE h.owner_id = $1 AND h.deleted_at IS NULL",
            )
            .bind(owner)
            .bind(other)
            .bind(member::FORMER)
            .fetch_all(db)
            .await
        };

        // What the blocker runs: the blocked person is put out.
        for space_id in rooms_with(blocker, blocked).await? {
            self.remove_from_space(space_id, &[blocked], leave::REMOVED).await?;
        }
        let mut tx = db.begin().await?;
        sqlx::query(
            "UPDATE space_invites i SET revoked_at = now() FROM spaces s
             WHERE s.id = i.space_id AND s.host_id = $1 AND i.user_id = $2 AND i.revoked_at IS NULL",
        )
        .bind(blocker)
        .bind(blocked)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE space_participants p SET role = $3 FROM spaces s
             WHERE s.id = p.space_id AND s.host_id = $1 AND p.user_id = $2 AND p.left_at IS NOT NULL AND p.role = $4",
        )
        .bind(blocker)
        .bind(blocked)
        .bind(role::LISTENER)
        .bind(role::COHOST)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        for hangout_id in hangouts_with(blocker, blocked).await? {
            self.drop_from_hangout(hangout_id, blocked).await?;
        }

        // What the blocked person runs: the blocker walks away.
        for space_id in rooms_with(blocked, blocker).await? {
            self.remove_from_space(space_id, &[blocker], leave::LEFT).await?;
        }
        for hangout_id in hangouts_with(blocked, blocker).await? {
            self.drop_from_hangout(hangout_id, blocker).await?;
        }

        // Open invitations between the two to somebody else's hangout.
        let invitations: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT hangout_id, user_id FROM hangout_members
             WHERE state = $3 AND ((user_id = $1 AND invited_by = $2) OR (user_id = $2 AND invited_by = $1))",
        )
        .bind(blocker)
        .bind(blocked)
        .bind(member::INVITED)
        .fetch_all(db)
        .await?;
        for (hangout_id, invitee) in invitations {
            self.drop_from_hangout(hangout_id, invitee).await?;
        }
        // Either may follow a room of the other without being inside (an
        // upcoming room, a friends room): the guard decides for each.
        self.core.rt.revoke_prefix(blocker, "space").await;
        self.core.rt.revoke_prefix(blocked, "space").await;
        Ok(())
    }

    // ───────────────────────────── moderation ─────────────────────────────

    /// `admin.content.removed`: a moderator took a room or a hangout down.
    /// A live room ends, an upcoming one is called off, a hangout is
    /// closed. Subjects of other domains, unknown IDs and rooms that are
    /// over already are done.
    pub(crate) async fn on_content_removed(&self, removal: ContentRemoved) -> AppResult<()> {
        if removal.subject_type != SubjectType::SUBJECT_TYPE_SPACE as i32 {
            return Ok(());
        }
        let mut tx = self.core.db.begin().await?;
        let Ok(space) = lock_space(&mut tx, removal.subject_id).await else {
            drop(tx);
            return self.dissolve_hangout(removal.subject_id).await;
        };
        if space.is_live() {
            let finished = self.finish(&mut tx, &space, end::MODERATION).await?;
            tx.commit().await?;
            self.after_finish(&finished).await;
        } else if space.is_scheduled() {
            let cancelled = self.call_off(&mut tx, &space, end::MODERATION).await?;
            tx.commit().await?;
            self.push_space(&cancelled).await;
        }
        Ok(())
    }
}
