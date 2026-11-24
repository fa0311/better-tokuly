//! What people do with a note they can see: like it, renote it, and be
//! counted as having viewed it.
use std::time::Duration;

use tokuly_core::{analytics::AnalyticsEvent, outbound, prelude::*};
use tokuly_proto::{
    common::{PageInfo, UserSummary},
    notes::NoteCounters,
};
use tokuly_social::InteractionKind;

use crate::{
    Notes, copy,
    hydrate::may_renote,
    model::{NOTE_COLUMNS, NoteRow},
    visibility::{Viewer, not_hidden, sees_note},
};

/// Likes and renotes one user may make per minute.
const REACTIONS_PER_MINUTE: u32 = 120;
/// `RecordNoteViews` calls one viewer may make per minute.
const VIEW_REPORTS_PER_MINUTE: u32 = 120;
/// Most notes in one `RecordNoteViews` or `BatchGetNotes` call.
pub const MAX_NOTES_PER_CALL: usize = 100;
/// How long a like or renote of the same note by the same person is not
/// announced again (liking, unliking and liking again is not three notifications).
const RENOTIFY_AFTER: Duration = Duration::from_secs(24 * 3600);

/// The two reactions a note keeps a row per user for. They work the same
/// way; only the table, the counter and the words differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reaction {
    Like,
    Renote,
}

impl Reaction {
    fn table(self) -> &'static str {
        match self {
            Self::Like => "note_likes",
            Self::Renote => "note_renotes",
        }
    }

    fn counter(self) -> &'static str {
        match self {
            Self::Like => "like_count",
            Self::Renote => "renote_count",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Like => "like",
            Self::Renote => "renote",
        }
    }
}

impl Notes {
    /// Records `user_id`'s like or renote of a note they can see. Doing it
    /// twice changes nothing. Returns the note's counters afterwards.
    pub(crate) async fn react(&self, user_id: Uuid, note_id: Uuid, reaction: Reaction) -> AppResult<NoteCounters> {
        let core = &self.core;
        core.cache.rate_limit(&format!("notes:react:{user_id}"), REACTIONS_PER_MINUTE, Duration::from_secs(60)).await?;
        let viewer = Viewer::load(core, Some(user_id)).await?;
        let note = self.require_note(&viewer, note_id).await?;
        if reaction == Reaction::Renote {
            // A renote puts the note in front of the renoter's friends and
            // followers; timelines still show it only to people in the
            // note's own audience, but only notes everyone may see are
            // passed on by other people.
            ensure!(
                may_renote(&viewer, &note),
                precondition,
                "NOTE_NOT_RENOTABLE",
                "only notes everyone may see can be renoted by others, and private notes not at all"
            );
        }
        let (table, counter) = (reaction.table(), reaction.counter());
        // Who to name in the notification, looked up before the transaction
        // opens: while it holds its connection nothing waits for a second one.
        let actor = match note.author_id == user_id {
            true => None,
            false => Some(tokuly_identity::summary(core, user_id).await?),
        };
        let mut tx = core.db.begin().await?;
        let added = sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO {table} (note_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING"
        )))
        .bind(note.id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0;
        if !added {
            return Ok(note.counters());
        }
        let sql = format!("UPDATE notes AS n SET {counter} = n.{counter} + 1 WHERE n.id = $1 RETURNING {NOTE_COLUMNS}");
        let note: NoteRow = sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(note.id).fetch_one(&mut *tx).await?;
        if let Some(actor) = &actor {
            let marker = format!("notes:notified:{}:{}:{user_id}", reaction.name(), note.id);
            if core.cache.set_nx(&marker, "1", RENOTIFY_AFTER).await.unwrap_or(true) {
                let request = match reaction {
                    Reaction::Like => copy::like(actor, user_id, &note),
                    Reaction::Renote => copy::renote(actor, user_id, &note),
                };
                outbound::notify(&mut *tx, &request).await?;
            }
        }
        tx.commit().await?;

        let counters = note.counters();
        self.push_counters([counters.clone()]).await;
        self.record_interaction(user_id, note.author_id, InteractionKind::MapReaction).await;
        core.analytics
            .track(
                AnalyticsEvent::new(&format!("note.{}", reaction.name()))
                    .user(Some(user_id))
                    .subject("note", note.id)
                    .owner(note.author_id),
            )
            .await;
        Ok(counters)
    }

    /// Takes `user_id`'s like or renote back. Succeeds when there was none.
    ///
    /// A reaction can be taken back from a note the user can no longer see
    /// (its author made it private since); the counters are then not returned.
    pub(crate) async fn unreact(&self, user_id: Uuid, note_id: Uuid, reaction: Reaction) -> AppResult<NoteCounters> {
        // Not rate-limited: there is never more to take back than was given.
        let core = &self.core;
        let viewer = Viewer::load(core, Some(user_id)).await?;
        let visible = self.find_note(&viewer, note_id).await?;
        let (table, counter) = (reaction.table(), reaction.counter());
        let mut tx = core.db.begin().await?;
        let removed =
            sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table} WHERE note_id = $1 AND user_id = $2")))
                .bind(note_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?
                .rows_affected()
                > 0;
        if !removed {
            return visible.map(|note| note.counters()).or_not_found("NOTE_NOT_FOUND", "note not found");
        }
        let sql = format!(
            "UPDATE notes AS n SET {counter} = GREATEST(n.{counter} - 1, 0) WHERE n.id = $1 RETURNING {NOTE_COLUMNS}"
        );
        let note: NoteRow = sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(note_id).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        let counters = note.counters();
        self.push_counters([counters.clone()]).await;
        Ok(if visible.is_some() { counters } else { NoteCounters::default() })
    }

    /// The people who liked or renoted a note, most recent first, without
    /// anyone hidden from the viewer by a block.
    pub(crate) async fn reactors(
        &self,
        viewer: &Viewer,
        note_id: Uuid,
        reaction: Reaction,
        page: &Page,
    ) -> AppResult<(Vec<UserSummary>, PageInfo)> {
        self.require_note(viewer, note_id).await?;
        let cursor: Option<Uuid> = page.decode()?;
        let sql = format!(
            "SELECT r.id, r.user_id FROM {table} r
             JOIN notes n ON n.id = r.note_id
             JOIN users u ON u.id = r.user_id
             WHERE r.note_id = $7 AND ($8::uuid IS NULL OR r.id < $8)
               AND {visible} AND {not_hidden} AND u.banned_at IS NULL AND u.deleted_at IS NULL
             ORDER BY r.id DESC LIMIT $9",
            table = reaction.table(),
            visible = sees_note("n"),
            not_hidden = not_hidden("r.user_id"),
        );
        let mut rows: Vec<(Uuid, Uuid)> =
            viewer.query_as(sql).bind(note_id).bind(cursor).bind(page.fetch()).fetch_all(&self.core.db).await?;
        let info = page.finish(&mut rows, |row| row.0);
        let user_ids: Vec<Uuid> = rows.iter().map(|row| row.1).collect();
        Ok((tokuly_identity::summaries_ordered(&self.core, &user_ids).await?, info))
    }

    /// Counts views of notes that were shown on screen. Each viewer counts
    /// once per note within the view window; authors never count towards
    /// their own notes; notes the viewer may not see are ignored.
    ///
    /// `client_ip` identifies anonymous viewers. Views are a statistic, not
    /// an action: a viewer who reports too often is ignored, not refused.
    pub(crate) async fn record_views(&self, viewer: &Viewer, client_ip: &str, note_ids: &[Uuid]) -> AppResult<()> {
        let who = match viewer.id {
            Some(user_id) => user_id.simple().to_string(),
            None if !client_ip.is_empty() => format!("ip:{client_ip}"),
            None => return Ok(()),
        };
        if note_ids.is_empty()
            || self
                .core
                .cache
                .rate_limit(&format!("notes:views:{who}"), VIEW_REPORTS_PER_MINUTE, Duration::from_secs(60))
                .await
                .is_err()
        {
            return Ok(());
        }
        let mut unique = note_ids.to_vec();
        unique.sort();
        unique.dedup();
        // One marker per viewer and note; only the first report in the window sets it.
        let window = self.cfg.view_window_hours.max(1) * 3600;
        let mut pipeline = redis::pipe();
        for note_id in &unique {
            pipeline
                .cmd("SET")
                .arg(format!("notes:view:{}:{who}", note_id.simple()))
                .arg(1)
                .arg("NX")
                .arg("EX")
                .arg(window);
        }
        let fresh: Vec<Option<String>> = pipeline.query_async(&mut self.core.cache.conn()).await?;
        let counted: Vec<Uuid> =
            unique.into_iter().zip(fresh).filter(|(_, set)| set.is_some()).map(|(note_id, _)| note_id).collect();
        if counted.is_empty() {
            return Ok(());
        }
        let sql = format!(
            "UPDATE notes n SET view_count = n.view_count + 1
             WHERE n.id = ANY($7) AND n.author_id IS DISTINCT FROM $1 AND {}",
            sees_note("n")
        );
        let sql = format!("WITH counted AS ({sql} RETURNING 1) SELECT count(*) FROM counted");
        viewer.query_scalar::<i64>(sql).bind(&counted).fetch_one(&self.core.db).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn likes_and_renotes_use_their_own_tables_and_counters() {
        assert_eq!(
            (Reaction::Like.table(), Reaction::Like.counter(), Reaction::Like.name()),
            ("note_likes", "like_count", "like")
        );
        assert_eq!(
            (Reaction::Renote.table(), Reaction::Renote.counter(), Reaction::Renote.name()),
            ("note_renotes", "renote_count", "renote")
        );
    }
}
