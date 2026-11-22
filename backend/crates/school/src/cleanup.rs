//! Reactions to what happens in other domains: deleted accounts, ended
//! friendships, blocks and moderation takedowns.
//!
//! Every reaction is idempotent — events are delivered at least once.
use tokuly_core::{events::Event, prelude::*};
use tokuly_identity::maintenance::UserRef;
use tokuly_proto::{common::SubjectType, school::RoomCloseReason};
use tokuly_social::events::{BlockChanged, FriendRemoved};

use crate::{
    School,
    events::{CONTENT_REMOVED, ContentRemoved, FRIEND_REMOVED, USER_BLOCKED, USER_DELETED},
    rooms::{self, Leave},
    study::sessions::{ENDED, End},
};

impl School {
    /// Reacts to one event from another domain. Public so tests can deliver
    /// events directly (no relay runs in tests).
    pub async fn handle_event(&self, event: &Event) -> AppResult<()> {
        match event.topic.as_str() {
            USER_DELETED => self.forget_user(event.data::<UserRef>()?.user_id).await,
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
        sqlx::query(
            "DELETE FROM school_term_shares s USING school_terms t
             WHERE s.term_id = t.id AND ((t.user_id = $1 AND s.user_id = $2) OR (t.user_id = $2 AND s.user_id = $1))",
        )
        .bind(a)
        .bind(b)
        .execute(&core.db)
        .await?;
        sqlx::query(
            "DELETE FROM school_room_invites i USING school_rooms r
             WHERE i.room_id = r.id AND ((r.host_id = $1 AND i.user_id = $2) OR (r.host_id = $2 AND i.user_id = $1))",
        )
        .bind(a)
        .bind(b)
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
        Ok(())
    }

    /// Removes everything this domain holds about a deleted account: their
    /// rooms close, their session ends, and their timetables, study log,
    /// exams, assignments and settings are erased.
    async fn forget_user(&self, user_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        let at = self.now();
        let hosted: Vec<Uuid> =
            sqlx::query_scalar("SELECT id FROM school_rooms WHERE host_id = $1 AND closed_at IS NULL")
                .bind(user_id)
                .fetch_all(&core.db)
                .await?;
        for room_id in hosted {
            self.close_room(room_id, RoomCloseReason::ROOM_CLOSE_REASON_REMOVED, None, at).await?;
        }
        // Whatever they had going ends the way a vanished client's does, so
        // a room they were in is told that they left.
        let mut tx = core.db.begin().await?;
        let gone = match self.lock_active(&mut tx, user_id).await? {
            Some((session, room)) => {
                Some(self.end_locked(&mut tx, session, room, End::Server, Leave::Removed, at).await?)
            }
            None => None,
        };
        tx.commit().await?;
        if let Some(gone) = &gone
            && let Some(exit) = &gone.exit
        {
            let session = &gone.ended.session;
            let proto = session.to_proto(None, &session.own_timer(), at, self.idle_limit());
            self.announce_exit(user_id, exit, &proto, at).await;
        }

        // Each statement removes the rows in which the user is a party; rows
        // hanging off them (segments, grid classes, invitations...) go along.
        const DELETES: &[&str] = &[
            "UPDATE school_study_sessions s SET cheer_count = (SELECT count(*) FROM school_study_cheers c WHERE c.session_id = s.id AND c.from_user_id <> $1)
             WHERE s.id IN (SELECT session_id FROM school_study_cheers WHERE from_user_id = $1)",
            "DELETE FROM school_study_cheers WHERE from_user_id = $1 OR to_user_id = $1",
            "DELETE FROM school_study_sessions WHERE user_id = $1",
            "DELETE FROM school_room_invites WHERE user_id = $1",
            "DELETE FROM school_room_bans WHERE user_id = $1",
            "DELETE FROM school_rooms WHERE host_id = $1",
            "DELETE FROM school_exams WHERE user_id = $1",
            "DELETE FROM school_assignments WHERE user_id = $1",
            "DELETE FROM school_term_shares WHERE user_id = $1",
            "DELETE FROM school_terms WHERE user_id = $1",
            "UPDATE school_terms SET copied_from_user_id = NULL WHERE copied_from_user_id = $1",
            "DELETE FROM school_subjects WHERE user_id = $1",
            "DELETE FROM school_settings WHERE user_id = $1",
        ];
        let mut tx = core.db.begin().await?;
        for delete in DELETES {
            sqlx::query(*delete).bind(user_id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }
}
