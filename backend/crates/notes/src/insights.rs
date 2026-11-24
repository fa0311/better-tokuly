//! What the author learns about one of their notes (impressions by hour,
//! engagement), and the trending notes everyone can read.
use chrono::{DateTime, Utc};
use tokuly_core::prelude::*;
use tokuly_proto::notes::{GetNoteAnalyticsResponse, HourlyCount, Note};

use crate::{
    Notes,
    hydrate::EntryRow,
    model::{NOTE_COLUMNS, NoteRow},
    mutes::TimelineFilter,
    visibility::{Viewer, listed, sees_note},
};

/// How far back the impressions by hour go.
const ANALYTICS_HOURS: i64 = 48;

#[derive(Debug, clickhouse::Row, serde::Deserialize)]
struct HourRow {
    hour: u32,
    count: u64,
}

impl Notes {
    /// The numbers of one of `user_id`'s notes. Impressions by hour come
    /// from the analytics store; when it cannot be reached the counters are
    /// still returned (without the hours).
    pub(crate) async fn note_analytics(&self, user_id: Uuid, note_id: Uuid) -> AppResult<GetNoteAnalyticsResponse> {
        let note = self.own_note(user_id, note_id).await?;
        let counters = note.counters();
        let engagements = note.like_count + note.renote_count + note.reply_count + note.quote_count + note.clip_count;
        let impressions = note.view_count;
        let hours = self
            .core
            .analytics
            .ch
            .query(
                "SELECT toUInt32(toUnixTimestamp(toStartOfHour(at))) AS hour, count() AS count FROM events
                 WHERE owner_id = toUUID(?) AND name = 'note.view' AND subject_id = toUUID(?)
                   AND at >= now() - toIntervalHour(?)
                 GROUP BY hour ORDER BY hour",
            )
            .bind(user_id.to_string())
            .bind(note.id.to_string())
            .bind(ANALYTICS_HOURS)
            .fetch_all::<HourRow>()
            .await;
        let impressions_by_hour = match hours {
            Ok(rows) => rows
                .into_iter()
                .filter_map(|row| {
                    let hour: DateTime<Utc> = DateTime::from_timestamp(i64::from(row.hour), 0)?;
                    Some(HourlyCount { hour: ts(hour), count: row.count as i64, ..Default::default() })
                })
                .collect(),
            Err(e) => {
                tracing::warn!(error = %e, "reading note impressions by hour failed");
                Vec::new()
            }
        };
        Ok(GetNoteAnalyticsResponse {
            counters: counters.into(),
            impressions,
            engagements,
            engagement_rate: if impressions > 0 { engagements as f64 / impressions as f64 } else { 0.0 },
            impressions_by_hour,
            ..Default::default()
        })
    }

    /// The trending notes of a language ('' = all) the viewer may see,
    /// without what they muted, best first.
    pub(crate) async fn trending_notes(
        &self,
        viewer: &Viewer,
        language: &str,
        limit: i64,
    ) -> AppResult<(Vec<Note>, Option<DateTime<Utc>>)> {
        let sql = format!(
            "SELECT n.id AS entry_id, NULL::uuid AS renoter_id, NULL::timestamptz AS renoted_at, {NOTE_COLUMNS}, t.computed_at
             FROM note_trends t JOIN notes n ON n.id = t.note_id
             WHERE t.language = $7 AND {listed} AND {visible}
             ORDER BY t.rank LIMIT $8",
            listed = listed("n"),
            visible = sees_note("n"),
        );
        #[derive(sqlx::FromRow)]
        struct Trend {
            #[sqlx(flatten)]
            entry: EntryRow,
            computed_at: DateTime<Utc>,
        }
        let rows: Vec<Trend> = viewer.query_as(sql).bind(language).bind(limit).fetch_all(&self.core.db).await?;
        let computed_at = rows.first().map(|row| row.computed_at);
        let mut entries: Vec<EntryRow> = rows.into_iter().map(|row| row.entry).collect();
        viewer.drop_newly_locked(&self.core, &mut entries, |row| &row.note).await?;
        TimelineFilter::load(&self.core, viewer).await?.apply(&mut entries);
        let notes: Vec<NoteRow> = entries.into_iter().map(|entry| entry.note).collect();
        Ok((self.hydrate(viewer, &notes).await?, computed_at))
    }
}
