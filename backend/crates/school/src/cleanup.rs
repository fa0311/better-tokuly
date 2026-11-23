//! Reactions to what happens in other domains: deleted and suspended
//! accounts, ended friendships, blocks and moderation takedowns.
//!
//! Events are delivered at least once and not always in order, so every
//! reaction gives the same result when it runs twice, and the ones that
//! depend on a state that can change back (a friendship, a block, a
//! suspension) look at that state as it is now rather than trusting the
//! event: an old "friendship ended" arriving after the two made up must not
//! take away what they share today.
use tokuly_core::{events::Event, prelude::*};
use tokuly_identity::maintenance::UserRef;
use tokuly_proto::{common::SubjectType, school::RoomCloseReason};
use tokuly_social::{
    events::{BlockChanged, FriendRemoved},
    graph,
};

use crate::{
    School,
    events::{CONTENT_REMOVED, ContentRemoved, FRIEND_REMOVED, USER_BANNED, USER_BLOCKED, USER_DELETED},
    rooms::{self, Leave},
    study::sessions::{ENDED, End},
};

impl School {
    /// Reacts to one event from another domain. Public so tests can deliver
    /// events directly (no relay runs in tests).
    pub async fn handle_event(&self, event: &Event) -> AppResult<()> {
        match event.topic.as_str() {
            USER_DELETED => self.forget_user(event.data::<UserRef>()?.user_id).await,
            USER_BANNED => self.suspend_user(event.data::<UserRef>()?.user_id).await,
            FRIEND_REMOVED => {
                let removed: FriendRemoved = event.data()?;
                self.separate(removed.user_id, removed.friend_id, None).await
            }
            USER_BLOCKED => {
                let block: BlockChanged = event.data()?;
                self.separate(block.user_id, block.blocked_id, Some(block.blocked_id)).await
            }
            CONTENT_REMOVED => {
                let removed: ContentRemoved = event.data()?;
                if removed.subject_type == SubjectType::SUBJECT_TYPE_STUDY_ROOM as i32 {
                    self.close_room(removed.subject_id, RoomCloseReason::ROOM_CLOSE_REASON_REMOVED, None, self.now())
                        .await?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Two people stopped being friends (or one blocked the other): neither
    /// stays in the other's chosen timetable audience, invitations between
    /// them are withdrawn, and neither stays in a room of the other's that
    /// took the friendship to enter. After a block — `blocked` names who was
    /// blocked — they also leave each other's public rooms, and the blocked
    /// one leaves any room the two are in together.
    async fn separate(&self, a: Uuid, b: Uuid, blocked: Option<Uuid>) -> AppResult<()> {
        let core = &self.core;
        let at = self.now();
        // A late event: the two are friends (again). What they share now was
        // set up after the friendship this event is about ended.
        if graph::are_friends(core, a, b).await? {
            return Ok(());
        }
        // Likewise a block that was lifted since has nothing left to enforce
        // beyond the ended friendship.
        let blocked = match blocked {
            Some(blocked) if graph::is_blocked_either(core, a, b).await? => Some(blocked),
            _ => None,
        };

        sqlx::query(
            "DELETE FROM school_term_shares s USING school_terms t
             WHERE s.term_id = t.id AND ((t.user_id = $1 AND s.user_id = $2) OR (t.user_id = $2 AND s.user_id = $1))",
        )
        .bind(a)
        .bind(b)
        .execute(&core.db)
        .await?;
        // Withdrawn, not erased: the invitee can still report what they were sent.
        sqlx::query(
            "UPDATE school_room_invites i SET withdrawn_at = $3 FROM school_rooms r
             WHERE i.room_id = r.id AND i.withdrawn_at IS NULL
               AND ((r.host_id = $1 AND i.user_id = $2) OR (r.host_id = $2 AND i.user_id = $1))",
        )
        .bind(a)
        .bind(b)
        .bind(at)
        .execute(&core.db)
        .await?;

        // Rooms one of them hosts with the other inside.
        let hosted: Vec<(Uuid, Uuid)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT r.id, s.user_id FROM school_rooms r
             JOIN school_study_sessions s ON s.room_id = r.id AND s.state <> {ENDED}
             WHERE r.closed_at IS NULL AND ($3 OR r.audience <> {})
               AND ((r.host_id = $1 AND s.user_id = $2) OR (r.host_id = $2 AND s.user_id = $1))",
            rooms::AUDIENCE_PUBLIC
        )))
        .bind(a)
        .bind(b)
        .bind(blocked.is_some())
        .fetch_all(&core.db)
        .await?;
        for (room_id, guest) in hosted {
            self.remove_from_room(room_id, guest, false, at).await?;
        }
        if let Some(blocked) = blocked {
            let blocker = if blocked == a { b } else { a };
            // Rooms of other people that the two are in together.
            let shared: Vec<Uuid> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT mine.room_id FROM school_study_sessions mine
                 JOIN school_study_sessions theirs ON theirs.room_id = mine.room_id AND theirs.state <> {ENDED} AND theirs.user_id = $2
                 WHERE mine.user_id = $1 AND mine.state <> {ENDED} AND mine.room_id IS NOT NULL"
            )))
            .bind(blocked)
            .bind(blocker)
            .fetch_all(&core.db)
            .await?;
            for room_id in shared {
                self.remove_from_room(room_id, blocked, false, at).await?;
            }
        }
        // Friends rooms of either, and rooms either follows without being
        // inside: the guard decides for each subscription.
        core.rt.revoke_prefix(a, "study").await;
        core.rt.revoke_prefix(b, "study").await;
        Ok(())
    }

    /// Takes an account out of everything live: the rooms it hosts close
    /// (everyone inside is out, their time logged), and its own session ends
    /// the way a vanished client's does, so a room it was in is told.
    async fn leave_everything(&self, user_id: Uuid, at: Time) -> AppResult<()> {
        let core = &self.core;
        let hosted: Vec<Uuid> =
            sqlx::query_scalar("SELECT id FROM school_rooms WHERE host_id = $1 AND closed_at IS NULL")
                .bind(user_id)
                .fetch_all(&core.db)
                .await?;
        for room_id in hosted {
            self.close_room(room_id, RoomCloseReason::ROOM_CLOSE_REASON_REMOVED, None, at).await?;
        }
        let mut tx = core.db.begin().await?;
        let gone = match self.lock_active(&mut tx, user_id).await? {
            Some((session, room)) => {
                Some(self.end_locked(&mut tx, session, room, End::Server, Leave::Removed, at).await?)
            }
            None => None,
        };
        tx.commit().await?;
        if let Some(gone) = &gone {
            self.announce_gone(gone, at).await;
        }
        Ok(())
    }

    /// A suspended account stops being present: its rooms close and its
    /// session ends. Nothing is deleted — what it shared is simply shown to
    /// nobody while the suspension lasts (every read checks the account).
    async fn suspend_user(&self, user_id: Uuid) -> AppResult<()> {
        // A late event: the suspension may have been lifted since.
        let suspended: Option<bool> =
            sqlx::query_scalar("SELECT banned_at IS NOT NULL OR deleted_at IS NOT NULL FROM users WHERE id = $1")
                .bind(user_id)
                .fetch_optional(&self.core.db)
                .await?;
        if suspended != Some(true) {
            return Ok(());
        }
        self.leave_everything(user_id, self.now()).await
    }

    /// Removes everything this domain holds about a deleted account: their
    /// rooms close, their session ends, and their timetables, study log,
    /// exams, assignments and settings are erased.
    async fn forget_user(&self, user_id: Uuid) -> AppResult<()> {
        self.leave_everything(user_id, self.now()).await?;

        // Each statement removes the rows in which the user is a party; rows
        // hanging off them (segments, grid classes, invitations...) go along.
        const DELETES: &[&str] = &[
            "UPDATE school_study_sessions s SET cheer_count = (SELECT count(*) FROM school_study_cheers c WHERE c.session_id = s.id AND c.from_user_id <> $1)
             WHERE s.id IN (SELECT session_id FROM school_study_cheers WHERE from_user_id = $1)",
            "DELETE FROM school_study_cheers WHERE from_user_id = $1 OR to_user_id = $1",
            "DELETE FROM school_study_sessions WHERE user_id = $1",
            "DELETE FROM school_room_invites WHERE user_id = $1",
            "DELETE FROM school_room_bans WHERE user_id = $1",
            "DELETE FROM school_room_visits WHERE user_id = $1",
            "DELETE FROM school_rooms WHERE host_id = $1",
            "DELETE FROM school_exams WHERE user_id = $1",
            "DELETE FROM school_assignments WHERE user_id = $1",
            "DELETE FROM school_term_shares WHERE user_id = $1",
            "DELETE FROM school_terms WHERE user_id = $1",
            "UPDATE school_terms SET copied_from_user_id = NULL WHERE copied_from_user_id = $1",
            "DELETE FROM school_subjects WHERE user_id = $1",
            "DELETE FROM school_settings WHERE user_id = $1",
        ];
        let mut tx = self.core.db.begin().await?;
        for delete in DELETES {
            sqlx::query(*delete).bind(user_id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }
}
