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

/// Takes `tags` off a note (an edit removed them), and out of the listed
/// counts if the note is listed.
pub(crate) async fn unlink(
    tx: &mut Transaction<'_, Postgres>,
    note_id: Uuid,
    tags: &[String],
    listed: bool,
) -> AppResult<()> {
    if tags.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "DELETE FROM note_hashtags nh USING hashtags h WHERE nh.hashtag_id = h.id AND nh.note_id = $1 AND h.tag = ANY($2)",
    )
    .bind(note_id)
    .bind(tags)
    .execute(&mut **tx)
    .await?;
    if listed {
        adjust(tx, tags, -1).await?;
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

/// SQL: who a use is counted as, for the account in alias `u`: itself, or —
/// for an account younger than `$6` days — the day it joined, so a crowd
/// of accounts made at once counts as one person.
fn who_counts(u: &str) -> String {
    format!(
        "CASE WHEN {u}.created_at > now() - make_interval(days => $6::int) THEN 'joined:' || to_char({u}.created_at, 'YYYY-MM-DD')
              ELSE {u}.id::text END"
    )
}

/// SQL: the account in alias `u` counts towards trends at all: in good
/// standing and at least `min_age_hours` old.
fn counts(u: &str, min_age_hours: i64) -> String {
    format!(
        "{u}.banned_at IS NULL AND {u}.deleted_at IS NULL AND {u}.created_at <= now() - make_interval(hours => {})",
        min_age_hours.max(0)
    )
}

/// The current trending ranking, best first, and when it was computed.
pub(crate) async fn trending(
    core: &Core,
    language: &str,
    limit: i64,
) -> AppResult<(Vec<TrendingHashtag>, Option<DateTime<Utc>>)> {
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
         WHERE h.note_count > 0 AND t.language = $2
         ORDER BY t.rank LIMIT $1",
    )
    .bind(limit)
    .bind(language)
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
    /// A hashtag trends when enough different people used it in listed
    /// notes within the window; a note trends when enough different people
    /// liked, renoted, answered or quoted it. Each person counts once, by
    /// their latest use, and a use counts half after six hours — so one
    /// account cannot push a tag up alone, and yesterday's topic fades.
    /// Against a crowd of fresh accounts: accounts younger than
    /// `trend_min_account_age_hours` do not count, and all accounts younger
    /// than `trend_new_account_days` that joined on the same day count as
    /// one person. Rankings exist for all notes and per language.
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
        sqlx::query("DELETE FROM note_trends").execute(&mut *tx).await?;
        let floor = id_floor(since - chrono::Duration::hours(1));
        // Who counts (see `who_counts`): each use is counted for all notes
        // ('') and for its language, by person; a person's heat is their
        // latest use, halved every TREND_HALF_LIFE_HOURS.
        let sql = format!(
            "WITH uses AS (
                 SELECT nh.hashtag_id, l.language, {who} AS who, count(*) AS notes, max(n.created_at) AS latest
                 FROM note_hashtags nh
                 JOIN notes n ON n.id = nh.note_id
                 JOIN users u ON u.id = n.author_id
                 CROSS JOIN LATERAL (SELECT DISTINCT unnest(ARRAY['', n.language]) AS language) l
                 WHERE nh.note_id >= $1 AND n.created_at > $2 AND n.listed AND {counts}
                 GROUP BY 1, 2, 3
             ), scored AS (
                 SELECT hashtag_id, language, sum(notes)::bigint AS notes,
                        sum(power(0.5, extract(epoch FROM (now() - latest)) / 3600.0 / $3)) AS score
                 FROM uses GROUP BY hashtag_id, language HAVING count(*) >= $4
             ), ranked AS (
                 SELECT *, row_number() OVER (PARTITION BY language ORDER BY score DESC, notes DESC, hashtag_id) AS rank FROM scored
             )
             INSERT INTO hashtag_trends (language, hashtag_id, rank, score, recent_note_count, computed_at)
             SELECT language, hashtag_id, rank::int, score, notes, now() FROM ranked WHERE rank <= $5",
            who = who_counts("u"),
            counts = counts("u", self.cfg.trend_min_account_age_hours),
        );
        let trending = sqlx::query(sqlx::AssertSqlSafe(sql))
            // Note IDs are time-ordered: the index on them narrows the scan
            // to recent links (with an hour of slack for clock differences).
            .bind(floor)
            .bind(since)
            .bind(TREND_HALF_LIFE_HOURS)
            .bind(self.cfg.trend_min_authors)
            .bind(TREND_SIZE)
            .bind(self.cfg.trend_new_account_days)
            .execute(&mut *tx)
            .await?
            .rows_affected();

        // Notes: likes, renotes, replies and quotes of listed top-level
        // notes within the window, by people other than the author, counted
        // the same way.
        let sql = format!(
            "WITH acts AS (
                 SELECT note_id, user_id, created_at FROM note_likes WHERE id >= $1 AND created_at > $2
                 UNION ALL SELECT note_id, user_id, created_at FROM note_renotes WHERE id >= $1 AND created_at > $2
                 UNION ALL SELECT reply_to_id, author_id, created_at FROM notes WHERE id >= $1 AND created_at > $2 AND reply_to_id IS NOT NULL
                 UNION ALL SELECT quote_of_id, author_id, created_at FROM notes WHERE id >= $1 AND created_at > $2 AND quote_of_id IS NOT NULL
             ), people AS (
                 SELECT a.note_id, l.language, {who} AS who, max(a.created_at) AS latest
                 FROM acts a
                 JOIN notes t ON t.id = a.note_id AND t.listed AND t.reply_to_id IS NULL AND t.author_id <> a.user_id
                 JOIN users u ON u.id = a.user_id
                 CROSS JOIN LATERAL (SELECT DISTINCT unnest(ARRAY['', t.language]) AS language) l
                 WHERE {counts}
                   AND EXISTS (SELECT 1 FROM users au WHERE au.id = t.author_id AND au.banned_at IS NULL AND au.deleted_at IS NULL)
                 GROUP BY 1, 2, 3
             ), scored AS (
                 SELECT note_id, language, sum(power(0.5, extract(epoch FROM (now() - latest)) / 3600.0 / $3)) AS score
                 FROM people GROUP BY note_id, language HAVING count(*) >= $4
             ), ranked AS (
                 SELECT *, row_number() OVER (PARTITION BY language ORDER BY score DESC, note_id DESC) AS rank FROM scored
             )
             INSERT INTO note_trends (language, note_id, rank, score, computed_at)
             SELECT language, note_id, rank::int, score, now() FROM ranked WHERE rank <= $5",
            who = who_counts("u"),
            counts = counts("u", self.cfg.trend_min_account_age_hours),
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(floor)
            .bind(since)
            .bind(TREND_HALF_LIFE_HOURS)
            .bind(self.cfg.trend_min_authors)
            .bind(TREND_SIZE)
            .bind(self.cfg.trend_new_account_days)
            .execute(&mut *tx)
            .await?;

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
