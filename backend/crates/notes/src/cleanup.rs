//! What happens when things disappear or relationships break: an account
//! is erased, moderation takes content down, an upload is deleted by force,
//! one user blocks another. These are the reactions to the events the notes
//! domain consumes; every one is idempotent.
use tokuly_core::{outbound, prelude::*};
use tokuly_proto::{common::SubjectType, media::Attachment, notes::StoryItem};

use crate::{
    Notes,
    events::{ContentRemoved, RemovalReason},
    model::{NOTE_COLUMNS, NoteRow, STORY_COLUMNS, StoryRow, to_json},
    posting::search_doc,
};

/// Notes and stories deleted per transaction when an account is erased.
const ERASE_BATCH: i64 = 200;

impl Notes {
    /// `identity.user.deleted`: erases what the account left in this
    /// domain — its notes and stories with everything on them, its likes,
    /// renotes, story views and reactions (the counters they added go down
    /// again), its place in other people's mentions and close-friends
    /// audiences, its highlights and its agreement to the rules.
    ///
    /// Replies other people wrote to the erased notes stay; they show what
    /// they answered as unavailable.
    pub(crate) async fn on_user_deleted(&self, user_id: Uuid) -> AppResult<()> {
        let db = &self.core.db;
        loop {
            let mut tx = db.begin().await?;
            let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM notes WHERE author_id = $1 ORDER BY id LIMIT $2")
                .bind(user_id)
                .bind(ERASE_BATCH)
                .fetch_all(&mut *tx)
                .await?;
            if ids.is_empty() {
                break;
            }
            let removed = self.delete_notes(&mut tx, &ids, RemovalReason::AccountDeleted, Some(user_id)).await?;
            tx.commit().await?;
            self.announce_removed(removed).await;
        }
        loop {
            let mut tx = db.begin().await?;
            let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM stories WHERE owner_id = $1 ORDER BY id LIMIT $2")
                .bind(user_id)
                .bind(ERASE_BATCH)
                .fetch_all(&mut *tx)
                .await?;
            if ids.is_empty() {
                break;
            }
            // Not announced: the account's friendships are gone with it.
            self.delete_stories(&mut tx, &ids, RemovalReason::AccountDeleted, Some(user_id)).await?;
            tx.commit().await?;
        }

        let mut tx = db.begin().await?;
        // What the user did to other people's content. A user has at most
        // one like, renote and view per note or story, so each is worth one.
        sqlx::query(
            "WITH gone AS (DELETE FROM note_likes WHERE user_id = $1 RETURNING note_id)
             UPDATE notes n SET like_count = GREATEST(n.like_count - 1, 0) WHERE n.id IN (SELECT note_id FROM gone)",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "WITH gone AS (DELETE FROM note_renotes WHERE user_id = $1 RETURNING note_id)
             UPDATE notes n SET renote_count = GREATEST(n.renote_count - 1, 0) WHERE n.id IN (SELECT note_id FROM gone)",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "WITH gone AS (DELETE FROM story_views WHERE viewer_id = $1 RETURNING story_id, reaction)
             UPDATE stories s
             SET view_count = GREATEST(s.view_count - 1, 0),
                 reaction_count = GREATEST(s.reaction_count - (gone.reaction <> '')::int, 0)
             FROM gone WHERE s.id = gone.story_id",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        // Mentions of the user become plain text again.
        sqlx::query(
            "UPDATE notes n
             SET mentions = (SELECT COALESCE(jsonb_agg(m.value), '[]'::jsonb) FROM jsonb_array_elements(n.mentions) AS m(value)
                             WHERE m.value ->> 'userId' IS DISTINCT FROM $1::text)
             WHERE n.id IN (SELECT note_id FROM note_mentions WHERE user_id = $1)",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        // A direct note to the user is addressed to the others it names.
        sqlx::query("UPDATE notes SET direct_user_ids = array_remove(direct_user_ids, $1) WHERE direct_user_ids @> ARRAY[$1]::uuid[]")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        for sql in [
            "DELETE FROM note_authors WHERE user_id = $1",
            "DELETE FROM note_mentions WHERE user_id = $1",
            "DELETE FROM note_recipients WHERE user_id = $1",
            "DELETE FROM story_audience WHERE user_id = $1",
            "DELETE FROM story_highlights WHERE owner_id = $1",
            "DELETE FROM note_rule_agreements WHERE user_id = $1",
        ] {
            sqlx::query(sql).bind(user_id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        self.erase_lists_and_antennas(user_id).await?;
        Ok(())
    }

    /// `admin.content.removed`: a moderator took a note or a story down.
    /// It is deleted like its author would delete it; the admin domain has
    /// already told the author. Unknown or already removed subjects, and
    /// subject types of other domains, are done.
    pub(crate) async fn on_content_removed(&self, removal: ContentRemoved) -> AppResult<()> {
        let note = SubjectType::SUBJECT_TYPE_NOTE as i32;
        let story = SubjectType::SUBJECT_TYPE_STORY as i32;
        if removal.subject_type == note {
            let mut tx = self.core.db.begin().await?;
            let removed = self
                .delete_notes(&mut tx, &[removal.subject_id], RemovalReason::Moderation, removal.moderator_id)
                .await?;
            tx.commit().await?;
            self.announce_removed(removed).await;
        } else if removal.subject_type == story {
            let mut tx = self.core.db.begin().await?;
            let removed = self
                .delete_stories(&mut tx, &[removal.subject_id], RemovalReason::Moderation, removal.moderator_id)
                .await?;
            tx.commit().await?;
            self.announce_removed_stories(removed).await;
        }
        Ok(())
    }

    /// `media.asset.deleted`: an upload is gone. Notes and stories hold
    /// their uploads, so their owner cannot delete those and the media
    /// janitor leaves them alone; this only happens when an upload was
    /// deleted regardless of holders (a takedown).
    ///
    /// The picture or video is taken out of whatever shows it. A note left
    /// with nothing (no text, no stamp, no other media) and a story left
    /// without items are deleted.
    pub(crate) async fn on_asset_deleted(&self, asset_id: Uuid) -> AppResult<()> {
        let db = &self.core.db;
        let asset = asset_id.to_string();

        let sql = format!("SELECT {NOTE_COLUMNS} FROM notes n WHERE n.asset_ids @> ARRAY[$1]::uuid[] ORDER BY n.id");
        let notes: Vec<NoteRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(asset_id).fetch_all(db).await?;
        for note in notes {
            let remaining: Vec<Attachment> =
                note.attachments().into_iter().filter(|attachment| attachment.asset_id != asset).collect();
            // For the search document; fetched before the transaction opens.
            let author = tokuly_identity::summary(&self.core, note.author_id).await?;
            let mut tx = db.begin().await?;
            if remaining.is_empty() && note.text.is_empty() && note.stamp_id.is_none() {
                let removed = self.delete_notes(&mut tx, &[note.id], RemovalReason::MediaDeleted, None).await?;
                tx.commit().await?;
                self.announce_removed(removed).await;
                continue;
            }
            let sql = format!(
                "UPDATE notes AS n SET attachments = $2, asset_ids = array_remove(n.asset_ids, $3) WHERE n.id = $1 RETURNING {NOTE_COLUMNS}"
            );
            let updated: Option<NoteRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
                .bind(note.id)
                .bind(to_json(&remaining)?)
                .bind(asset_id)
                .fetch_optional(&mut *tx)
                .await?;
            if let Some(updated) = updated.filter(NoteRow::is_listed) {
                // The search result should not keep showing the removed picture.
                outbound::index(&mut *tx, search_doc(&updated, &author)).await?;
            }
            tx.commit().await?;
        }

        let sql = format!("SELECT {STORY_COLUMNS} FROM stories s WHERE s.asset_ids @> ARRAY[$1]::uuid[] ORDER BY s.id");
        let stories: Vec<StoryRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(asset_id).fetch_all(db).await?;
        for story in stories {
            let remaining: Vec<StoryItem> = story
                .items()
                .into_iter()
                .filter(|item| item.media.as_option().is_none_or(|media| media.asset_id != asset))
                .collect();
            let mut tx = db.begin().await?;
            if remaining.is_empty() {
                let removed = self.delete_stories(&mut tx, &[story.id], RemovalReason::MediaDeleted, None).await?;
                tx.commit().await?;
                self.announce_removed_stories(removed).await;
                continue;
            }
            sqlx::query("UPDATE stories SET items = $2, asset_ids = array_remove(asset_ids, $3) WHERE id = $1")
                .bind(story.id)
                .bind(to_json(&remaining)?)
                .bind(asset_id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        Ok(())
    }

    /// `social.user.blocked`: `blocker` blocked `blocked`.
    ///
    /// Nothing has to be hidden here: every read asks the social domain who
    /// is blocked, so the two stop seeing each other's notes and stories at
    /// once. What is removed is the one standing grant between them — being
    /// among the chosen friends of the other's close-friends stories — so
    /// it does not come back to life if they ever become friends again.
    pub(crate) async fn on_user_blocked(&self, blocker: Uuid, blocked: Uuid) -> AppResult<()> {
        sqlx::query(
            "DELETE FROM story_audience a USING stories s
             WHERE s.id = a.story_id AND ((s.owner_id = $1 AND a.user_id = $2) OR (s.owner_id = $2 AND a.user_id = $1))",
        )
        .bind(blocker)
        .bind(blocked)
        .execute(&self.core.db)
        .await?;
        self.sever_list_ties(blocker, blocked).await?;
        Ok(())
    }
}
