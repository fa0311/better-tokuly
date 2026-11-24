//! Hashtags: the registry, hashtag pages, trending and completion.
//!
//! A hashtag row exists once a short has carried the tag; `use_count`
//! counts the public shorts carrying it right now (kept exact by
//! [`crate::listing`]), and a hashtag with a count of zero is not shown
//! anywhere. Trending is computed by the ranking worker
//! ([`refresh_trending`]).
use tokuly_core::{
    outbound::{self, SearchDoc},
    prelude::*,
};
use tokuly_proto::{
    common::PageInfo,
    shorts::{Hashtag, HashtagSort},
};

use crate::{
    access::{self, Viewer},
    shorts::{ShortRow, short_columns},
    snapshot,
    sounds::like_literal,
    text,
};

/// How many hashtags trend at a time.
pub const TRENDING_SIZE: i64 = 50;
/// Channels that must carry a hashtag in active shorts before it can trend:
/// nobody trends a hashtag on their own.
pub const TREND_MIN_CHANNELS: i64 = 2;
/// Hashtag search documents refreshed per worker run.
const INDEX_BATCH: i64 = 500;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct HashtagRow {
    pub id: Uuid,
    pub tag: String,
    pub use_count: i64,
    pub view_count: i64,
    pub trend_score: f64,
    pub trend_rank: i32,
}

const HASHTAG_COLUMNS: &str = "h.id, h.tag, h.use_count, h.view_count, h.trend_score, h.trend_rank";

impl HashtagRow {
    /// The hashtag as a viewer sees it. `hidden` is what they do not get to
    /// see of it ([`hidden_uses`]): `(shorts, views of those shorts)`.
    pub fn to_proto(&self, hidden: Option<&(i64, i64)>) -> Hashtag {
        let (hidden_shorts, hidden_views) = hidden.copied().unwrap_or((0, 0));
        Hashtag {
            tag: self.tag.clone(),
            short_count: (self.use_count - hidden_shorts).max(0),
            view_count: (self.view_count - hidden_views).max(0),
            trending_rank: self.trend_rank,
            ..Default::default()
        }
    }

    /// Path of the hashtag page in the web app.
    pub fn path(&self) -> String {
        format!("/shorts/tags/{}", self.tag)
    }
}

/// What of each hashtag is hidden from `viewer`: the public shorts carrying
/// it whose owner is in a block with them, as `(shorts, their views)`.
/// Taken off the counts a viewer is shown, so that a count never
/// disagrees with the page behind it and gives a block away.
pub async fn hidden_uses(
    core: &Core,
    viewer: &Viewer,
    hashtag_ids: &[Uuid],
) -> AppResult<std::collections::HashMap<Uuid, (i64, i64)>> {
    if viewer.blocked.is_empty() || hashtag_ids.is_empty() {
        return Ok(Default::default());
    }
    let rows: Vec<(Uuid, i64, i64)> = sqlx::query_as(
        "SELECT u.hashtag_id, count(*), COALESCE(sum(s.view_count), 0)::bigint
         FROM short_hashtag_uses u JOIN shorts s ON s.id = u.short_id
         WHERE u.hashtag_id = ANY($1) AND s.owner_id = ANY($2)
         GROUP BY u.hashtag_id",
    )
    .bind(hashtag_ids)
    .bind(&viewer.blocked)
    .fetch_all(&core.db)
    .await?;
    Ok(rows.into_iter().map(|(id, shorts, views)| (id, (shorts, views))).collect())
}

/// The hashtags in `rows` as `viewer` sees them, leaving out those of which
/// nothing is left for them to see.
pub async fn to_protos(core: &Core, viewer: &Viewer, rows: &[HashtagRow]) -> AppResult<Vec<Hashtag>> {
    let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
    let hidden = hidden_uses(core, viewer, &ids).await?;
    Ok(rows.iter().map(|row| row.to_proto(hidden.get(&row.id))).filter(|hashtag| hashtag.short_count > 0).collect())
}

/// A hashtag as clients name it (with or without `#`), normalized.
pub fn normalize(input: &str) -> AppResult<String> {
    text::normalize_hashtag(input).ok_or_else(|| AppError::invalid("HASHTAG_INVALID", "that is not a hashtag"))
}

/// The hashtag `tag` (normalized), if any public short carries it.
pub async fn find(core: &Core, tag: &str) -> AppResult<Option<HashtagRow>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {HASHTAG_COLUMNS} FROM short_hashtags h WHERE h.tag = $1 AND h.use_count > 0"
    )))
    .bind(tag)
    .fetch_optional(&core.db)
    .await?)
}

/// The trending hashtags, the strongest first.
pub async fn trending(core: &Core, limit: i64) -> AppResult<Vec<HashtagRow>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {HASHTAG_COLUMNS} FROM short_hashtags h WHERE h.trend_rank > 0 AND h.use_count > 0
         ORDER BY h.trend_rank LIMIT $1"
    )))
    .bind(limit)
    .fetch_all(&core.db)
    .await?)
}

/// Hashtags starting with `prefix` (normalized), the most used first.
pub async fn suggest(core: &Core, prefix: &str, limit: i64) -> AppResult<Vec<HashtagRow>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {HASHTAG_COLUMNS} FROM short_hashtags h WHERE h.tag LIKE $1 AND h.use_count > 0
         ORDER BY h.use_count DESC, h.tag LIMIT $2"
    )))
    .bind(format!("{}%", like_literal(prefix)))
    .bind(limit)
    .fetch_all(&core.db)
    .await?)
}

/// Public shorts carrying a hashtag: the best ranked first (TOP) or the
/// newest first (RECENT).
pub async fn shorts_page(
    core: &Core,
    viewer: &Viewer,
    hashtag_id: Uuid,
    sort: HashtagSort,
    page: &Page,
) -> AppResult<(Vec<ShortRow>, PageInfo)> {
    if sort == HashtagSort::HASHTAG_SORT_RECENT {
        let before: Option<(Time, Uuid)> = page.decode()?;
        let mut rows: Vec<ShortRow> = viewer
            .query_as(concat!(
                "SELECT ",
                short_columns!(),
                " FROM short_hashtag_uses u JOIN shorts s ON s.id = u.short_id
              WHERE u.hashtag_id = $7 AND ",
                access::public!(),
                " AND ($8::timestamptz IS NULL OR (u.published_at, u.short_id) < ($8, $9))
              ORDER BY u.published_at DESC, u.short_id DESC LIMIT $10"
            ))
            .bind(hashtag_id)
            .bind(before.map(|(at, _)| at))
            .bind(before.map(|(_, id)| id))
            .bind(page.fetch())
            .fetch_all(&core.db)
            .await?;
        let info = page.finish(&mut rows, |row| (row.published_at.unwrap_or(row.created_at), row.id));
        viewer.drop_newly_locked(core, &mut rows, ShortRow::lock_facts).await?;
        return Ok((rows, info));
    }
    // The score moves with every ranking run: page on a snapshot of the order.
    let build = || async move {
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT s.id FROM short_hashtag_uses u JOIN shorts s ON s.id = u.short_id
             WHERE u.hashtag_id = $1 AND s.listed ORDER BY s.score DESC, s.id DESC LIMIT $2",
        )
        .bind(hashtag_id)
        .bind(snapshot::SNAPSHOT_SIZE)
        .fetch_all(&core.db)
        .await?;
        Ok(ids)
    };
    let (mut rows, info) =
        snapshot::read(core, &top_key(hashtag_id), page, build, |row| access::sees_public(viewer, &row.facts()))
            .await?;
    viewer.drop_newly_locked(core, &mut rows, ShortRow::lock_facts).await?;
    Ok((rows, info))
}

/// Where the snapshot of a hashtag's TOP order is kept.
pub fn top_key(hashtag_id: Uuid) -> String {
    format!("shorts:top:hashtag:{}", hashtag_id.simple())
}

/// Recomputes which hashtags trend. A hashtag's trend is the summed
/// `trending` (decayed recent engagement) of the public shorts carrying it;
/// the top [`TRENDING_SIZE`] carried by at least [`TREND_MIN_CHANNELS`]
/// channels are ranked. `view_count` is refreshed for every hashtag with
/// active shorts. Returns how many hashtags trend.
pub async fn refresh_trending(core: &Core) -> AppResult<u64> {
    let mut tx = core.db.begin().await?;
    // One refresh at a time (several instances, or a run that overlaps the next): two
    // refreshes update the same hashtag rows in different orders and deadlock.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('shorts:trending', 0))").execute(&mut *tx).await?;
    // Hashtags whose shorts have all gone quiet leave the ranking.
    sqlx::query(
        "UPDATE short_hashtags h SET trend_score = 0, trend_rank = 0
         WHERE (h.trend_rank > 0 OR h.trend_score > 0)
           AND NOT EXISTS (SELECT 1 FROM short_hashtag_uses u JOIN shorts s ON s.id = u.short_id
                           WHERE u.hashtag_id = h.id AND s.listed AND s.trending > 0)",
    )
    .execute(&mut *tx)
    .await?;
    let ranked: i64 = sqlx::query_scalar(
        "WITH active AS (
             SELECT u.hashtag_id, sum(s.trending) AS trend, count(DISTINCT s.channel_id) AS channels
             FROM shorts s JOIN short_hashtag_uses u ON u.short_id = s.id
             WHERE s.listed AND s.trending > 0
             GROUP BY u.hashtag_id
         ), ranked AS (
             SELECT hashtag_id, trend,
                    CASE WHEN channels >= $1
                         THEN row_number() OVER (PARTITION BY (channels >= $1) ORDER BY trend DESC, hashtag_id)
                         ELSE 0 END AS position
             FROM active
         ), written AS (
             UPDATE short_hashtags h
             SET trend_score = r.trend,
                 trend_rank = CASE WHEN r.position BETWEEN 1 AND $2 THEN r.position ELSE 0 END,
                 view_count = (SELECT COALESCE(sum(s.view_count), 0)::bigint
                               FROM short_hashtag_uses u JOIN shorts s ON s.id = u.short_id
                               WHERE u.hashtag_id = h.id)
             FROM ranked r WHERE r.hashtag_id = h.id
             RETURNING h.trend_rank
         )
         SELECT count(*) FROM written WHERE trend_rank > 0",
    )
    .bind(TREND_MIN_CHANNELS)
    .bind(TRENDING_SIZE)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(ranked as u64)
}

fn search_doc(hashtag: &HashtagRow) -> SearchDoc {
    SearchDoc {
        kind: "hashtag".to_owned(),
        id: hashtag.id,
        title: format!("#{}", hashtag.tag),
        text: String::new(),
        tags: vec![hashtag.tag.clone()],
        owner_id: None,
        public: hashtag.use_count > 0,
        image_url: None,
        path: hashtag.path(),
        created_at: now().timestamp(),
        score: hashtag.use_count as f64,
        extra: serde_json::json!({ "tag": hashtag.tag, "short_count": hashtag.use_count, "source": "shorts" }),
    }
}

/// Rewrites the search documents of hashtags whose count moved since they
/// were last indexed (a hashtag nothing public carries leaves the index).
/// Returns how many were written; call again while it returns a full batch.
pub async fn refresh_search(core: &Core) -> AppResult<u64> {
    let mut tx = core.db.begin().await?;
    let stale: Vec<HashtagRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {HASHTAG_COLUMNS} FROM short_hashtags h WHERE h.indexed_count <> h.use_count
         ORDER BY h.id LIMIT $1 FOR UPDATE SKIP LOCKED"
    )))
    .bind(INDEX_BATCH)
    .fetch_all(&mut *tx)
    .await?;
    for hashtag in &stale {
        outbound::index(&mut *tx, search_doc(hashtag)).await?;
        sqlx::query("UPDATE short_hashtags SET indexed_count = $2 WHERE id = $1")
            .bind(hashtag.id)
            .bind(hashtag.use_count)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(stale.len() as u64)
}

/// Marks a batch of hashtags (by ID after `after`) for re-indexing and
/// returns the last ID of the batch, or `None` when there are no more.
pub(crate) async fn mark_for_reindex(core: &Core, after: Option<Uuid>) -> AppResult<Option<Uuid>> {
    let last: Option<Uuid> = sqlx::query_scalar(
        "WITH batch AS (
             SELECT id FROM short_hashtags WHERE use_count > 0 AND ($1::uuid IS NULL OR id > $1) ORDER BY id LIMIT $2
         ), marked AS (
             UPDATE short_hashtags h SET indexed_count = -1 FROM batch WHERE h.id = batch.id RETURNING h.id
         )
         SELECT id FROM marked ORDER BY id DESC LIMIT 1",
    )
    .bind(after)
    .bind(INDEX_BATCH)
    .fetch_optional(&core.db)
    .await?;
    Ok(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashtags_are_named_with_or_without_the_sign() {
        assert_eq!(normalize("#Dance").unwrap(), "dance");
        assert_eq!(normalize("ダンス").unwrap(), "ダンス");
        assert_eq!(normalize("").unwrap_err().reason, "HASHTAG_INVALID");
        assert_eq!(normalize("two words").unwrap_err().reason, "HASHTAG_INVALID");
    }

    #[test]
    fn a_hashtag_without_public_shorts_leaves_the_index() {
        let mut hashtag = HashtagRow {
            id: Uuid::now_v7(),
            tag: "dance".into(),
            use_count: 3,
            view_count: 0,
            trend_score: 0.0,
            trend_rank: 0,
        };
        let doc = search_doc(&hashtag);
        assert!(doc.public);
        assert_eq!((doc.title.as_str(), doc.path.as_str(), doc.score), ("#dance", "/shorts/tags/dance", 3.0));
        hashtag.use_count = 0;
        assert!(!search_doc(&hashtag).public);
    }
}
