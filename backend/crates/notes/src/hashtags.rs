//! Hashtags: the directory of tags, how many public notes carry each, the
//! trending ranking and completion while typing.
//!
//! `hashtags.note_count` counts public notes only (it is what people see
//! next to a tag and what search ranks by), while `note_hashtags` links
//! every note to its tags, so a friends-only note still shows up for its
//! author's friends in the hashtag timeline.
//!
//! A hashtag is in the search index (`kind: "hashtag"`) for as long as at
//! least one public note carries it.
use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use tokuly_core::{
    outbound::{self, SearchDoc},
    prelude::*,
};
use tokuly_proto::notes::{Hashtag, TrendingHashtag};

use crate::{
    Notes,
    model::{hashtag_path, id_floor},
};

/// Age at which a use of a hashtag counts half towards the trend score.
const TREND_HALF_LIFE_HOURS: f64 = 6.0;
/// Hashtags kept in the ranking.
const TREND_SIZE: i64 = 50;
/// Hashtags re-sent to the search index per refresh.
const REINDEX_BATCH: i64 = 500;

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct HashtagRow {
    pub id: Uuid,
    pub tag: String,
    pub note_count: i64,
    pub created_at: DateTime<Utc>,
}

impl HashtagRow {
    pub fn to_proto(&self) -> Hashtag {
        Hashtag { tag: self.tag.clone(), note_count: self.note_count, ..Default::default() }
    }

    fn search_doc(&self) -> SearchDoc {
        SearchDoc {
            kind: "hashtag".to_owned(),
            id: self.id,
            title: format!("#{}", self.tag),
            text: String::new(),
            tags: vec![self.tag.clone()],
            owner_id: None,
            public: self.note_count > 0,
            image_url: None,
            path: hashtag_path(&self.tag),
            created_at: self.created_at.timestamp(),
            score: self.note_count as f64,
            extra: serde_json::json!({ "tag": self.tag, "note_count": self.note_count }),
        }
    }
}

/// Links a new note to its hashtags, creating tags seen for the first time,
/// and counts the note towards them if it is public.
pub(crate) async fn link(
    tx: &mut Transaction<'_, Postgres>,
    note_id: Uuid,
    tags: &[String],
    public: bool,
) -> AppResult<()> {
    if tags.is_empty() {
        return Ok(());
    }
    // Tags are written in a fixed order, so two notes sharing tags cannot
    // wait on each other's rows in a circle.
    let mut ordered = tags.to_vec();
    ordered.sort();
    ordered.dedup();
    let rows: Vec<HashtagRow> = sqlx::query_as(
        "INSERT INTO hashtags (tag, note_count)
         SELECT tag, $2 FROM unnest($1::text[]) AS t(tag) ORDER BY tag
         ON CONFLICT (tag) DO UPDATE SET note_count = hashtags.note_count + EXCLUDED.note_count
         RETURNING id, tag, note_count, created_at",
    )
    .bind(&ordered)
    .bind(i64::from(public))
    .fetch_all(&mut **tx)
    .await?;
    let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
    sqlx::query("INSERT INTO note_hashtags (hashtag_id, note_id) SELECT id, $2 FROM unnest($1::uuid[]) AS t(id) ON CONFLICT DO NOTHING")
        .bind(&ids)
        .bind(note_id)
        .execute(&mut **tx)
        .await?;
    if public {
        // A tag's first public note puts it into the search index.
        for row in rows.iter().filter(|row| row.note_count == 1) {
            index(tx, row).await?;
        }
    }
    Ok(())
}

/// Adds `delta` (per occurrence in `tags`) to the public note count of each
/// tag: public notes were deleted, or a note became or stopped being
/// public. Tags that gain their first public note enter the search index;
/// tags that lose their last one leave it.
pub(crate) async fn adjust(tx: &mut Transaction<'_, Postgres>, tags: &[String], delta: i64) -> AppResult<()> {
    if tags.is_empty() || delta == 0 {
        return Ok(());
    }
    #[derive(sqlx::FromRow)]
    struct Adjusted {
        #[sqlx(flatten)]
        row: HashtagRow,
        previous: i64,
    }
    let adjusted: Vec<Adjusted> = sqlx::query_as(
        "WITH locked AS (
             SELECT id, note_count FROM hashtags WHERE tag = ANY($1) ORDER BY tag FOR UPDATE
         ), change AS (
             SELECT tag, count(*) * $2 AS delta FROM unnest($1::text[]) AS t(tag) GROUP BY tag
         )
         UPDATE hashtags h SET note_count = GREATEST(h.note_count + change.delta, 0)
         FROM change, locked
         WHERE h.tag = change.tag AND locked.id = h.id
         RETURNING h.id, h.tag, h.note_count, h.created_at, locked.note_count AS previous",
    )
    .bind(tags)
    .bind(delta)
    .fetch_all(&mut **tx)
    .await?;
    for change in adjusted.iter().filter(|change| (change.previous == 0) != (change.row.note_count == 0)) {
        index(tx, &change.row).await?;
    }
    Ok(())
}

/// Sends a hashtag's document to the search index (or removes it when no
/// public note carries the tag) and remembers the count that was sent.
async fn index(tx: &mut Transaction<'_, Postgres>, row: &HashtagRow) -> AppResult<()> {
    outbound::index(&mut **tx, row.search_doc()).await?;
    sqlx::query("UPDATE hashtags SET indexed_note_count = note_count WHERE id = $1")
        .bind(row.id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// The hashtag with this (normalized) tag, if any note ever used it.
pub(crate) async fn find(core: &Core, tag: &str) -> AppResult<Option<HashtagRow>> {
    Ok(sqlx::query_as("SELECT id, tag, note_count, created_at FROM hashtags WHERE tag = $1")
        .bind(tag)
        .fetch_optional(&core.db)
        .await?)
}

/// Hashtags starting with `prefix` that public notes carry, most used first.
pub(crate) async fn complete(core: &Core, prefix: &str, limit: i64) -> AppResult<Vec<Hashtag>> {
    // `%` and `_` in what the user typed are literal characters, not wildcards.
    let pattern = format!("{}%", prefix.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
    let rows: Vec<HashtagRow> = sqlx::query_as(
        "SELECT id, tag, note_count, created_at FROM hashtags
         WHERE tag LIKE $1 AND note_count > 0
         ORDER BY note_count DESC, tag LIMIT $2",
    )
    .bind(pattern)
    .bind(limit)
    .fetch_all(&core.db)
    .await?;
    Ok(rows.iter().map(HashtagRow::to_proto).collect())
}

/// The current trending ranking, best first, and when it was computed.
pub(crate) async fn trending(core: &Core, limit: i64) -> AppResult<(Vec<TrendingHashtag>, Option<DateTime<Utc>>)> {
    #[derive(sqlx::FromRow)]
    struct Trend {
        tag: String,
        note_count: i64,
        rank: i32,
        recent_note_count: i64,
        computed_at: DateTime<Utc>,
    }
    let rows: Vec<Trend> = sqlx::query_as(
        "SELECT h.tag, h.note_count, t.rank, t.recent_note_count, t.computed_at
         FROM hashtag_trends t JOIN hashtags h ON h.id = t.hashtag_id
         WHERE h.note_count > 0 AND t.language = ''
         ORDER BY t.rank LIMIT $1",
    )
    .bind(limit)
    .fetch_all(&core.db)
    .await?;
    let computed_at = rows.first().map(|row| row.computed_at);
    let hashtags = rows
        .into_iter()
        .map(|row| TrendingHashtag {
            hashtag: Hashtag { tag: row.tag, note_count: row.note_count, ..Default::default() }.into(),
            rank: row.rank,
            recent_note_count: row.recent_note_count,
            ..Default::default()
        })
        .collect();
    Ok((hashtags, computed_at))
}

impl Notes {
    /// Recomputes the trending hashtags (the trending worker; public so
    /// tools and tests can run a pass). Returns how many hashtags trend.
    ///
    /// A hashtag trends when enough different people used it in public
    /// notes within the window. Each person counts once per hashtag, by
    /// their latest use, and a use counts half after six hours — so one
    /// account cannot push a tag up alone, and yesterday's topic fades.
    pub async fn refresh_trends(&self) -> AppResult<usize> {
        let since = Utc::now() - chrono::Duration::hours(self.cfg.trend_window_hours);
        let mut tx = self.core.db.begin().await?;
        // One refresh at a time, whichever worker process gets there first.
        let free: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended('notes:trends', 0))")
            .fetch_one(&mut *tx)
            .await?;
        if !free {
            return Ok(0);
        }
        sqlx::query("DELETE FROM hashtag_trends").execute(&mut *tx).await?;
        let sql = "WITH uses AS (
                 SELECT nh.hashtag_id, n.author_id, count(*) AS notes, max(n.created_at) AS latest
                 FROM note_hashtags nh
                 JOIN notes n ON n.id = nh.note_id
                 JOIN users u ON u.id = n.author_id
                 WHERE nh.note_id >= $1 AND n.created_at > $2 AND n.listed
                   AND u.banned_at IS NULL AND u.deleted_at IS NULL
                 GROUP BY nh.hashtag_id, n.author_id
             ), scored AS (
                 SELECT hashtag_id, sum(notes)::bigint AS notes,
                        sum(power(0.5, extract(epoch FROM (now() - latest)) / 3600.0 / $3)) AS score
                 FROM uses GROUP BY hashtag_id HAVING count(*) >= $4
             )
             INSERT INTO hashtag_trends (language, hashtag_id, rank, score, recent_note_count, computed_at)
             SELECT '', hashtag_id, (row_number() OVER (ORDER BY score DESC, notes DESC, hashtag_id))::int, score, notes, now()
             FROM scored ORDER BY score DESC, notes DESC, hashtag_id LIMIT $5";
        let trending = sqlx::query(sql)
            // Note IDs are time-ordered: the index on them narrows the scan
            // to recent links (with an hour of slack for clock differences).
            .bind(id_floor(since - chrono::Duration::hours(1)))
            .bind(since)
            .bind(TREND_HALF_LIFE_HOURS)
            .bind(self.cfg.trend_min_authors)
            .bind(TREND_SIZE)
            .execute(&mut *tx)
            .await?
            .rows_affected();

        // Search ranks hashtags by how much they are used; refresh the
        // documents of tags whose count moved since they were last sent.
        let stale: Vec<HashtagRow> = sqlx::query_as(
            "SELECT id, tag, note_count, created_at FROM hashtags
             WHERE note_count <> indexed_note_count ORDER BY id LIMIT $1 FOR UPDATE SKIP LOCKED",
        )
        .bind(REINDEX_BATCH)
        .fetch_all(&mut *tx)
        .await?;
        for row in &stale {
            index(&mut tx, row).await?;
        }
        tx.commit().await?;
        Ok(trending as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashtags_without_public_notes_leave_the_index() {
        let row = |note_count| HashtagRow { id: Uuid::now_v7(), tag: "春".into(), note_count, created_at: Utc::now() };
        let listed = row(3).search_doc();
        assert!(listed.public);
        assert_eq!(
            (listed.kind.as_str(), listed.title.as_str(), listed.path.as_str(), listed.score),
            ("hashtag", "#春", "/notes/tags/春", 3.0)
        );
        assert!(!row(0).search_doc().public);
    }
}
