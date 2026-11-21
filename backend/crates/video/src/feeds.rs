//! Discovery: the home feed, the subscriptions feed, trending and related
//! videos, plus the trending score the worker maintains.
//!
//! # Home feed
//!
//! Candidates come from five sources — new videos of subscribed channels,
//! uploads of friends' channels, public videos friends liked, what is
//! trending, and what is new — minus what the viewer finished, marked "not
//! interested" or cannot see. They are ranked by source, trending score and
//! freshness, then spread so one channel does not fill the screen. The
//! ranked list is computed once per visit and kept in Valkey for half an
//! hour; pages are slices of it, so scrolling never repeats or skips.
//!
//! # Trending
//!
//! `videos.trending_score` is the sum of each recent hour's
//! `views + 4 × likes`, halved for every `half_life_hours` of age
//! (`video_hourly_stats`, last seven days). The worker recomputes it every
//! few minutes; listings just sort by it.
use std::{collections::VecDeque, time::Duration};

use serde::{Deserialize, Serialize};
use tokuly_core::prelude::*;

use crate::{
    access::{Viewer, in_feed, public_video, watchable},
    videos::{VideoRow, video_columns},
};

/// How long a ranked home feed is kept for paging.
const HOME_TTL: Duration = Duration::from_secs(30 * 60);
/// Most videos one home feed visit can page through.
const HOME_MAX: usize = 400;
/// How far the trending listing can be paged.
pub const TRENDING_MAX_OFFSET: i64 = 1000;

#[derive(sqlx::FromRow)]
struct Candidate {
    #[sqlx(flatten)]
    video: VideoRow,
    boost: f64,
}

/// Ranking score of a home feed candidate: where it came from, how much it
/// trends, and how fresh it is (a new video starts with two points that
/// halve every five days).
pub fn home_score(boost: f64, trending_score: f64, age_days: f64) -> f64 {
    boost + 0.6 * (1.0 + trending_score.max(0.0)).ln() + 2.0 * 0.5_f64.powf(age_days.max(0.0) / 5.0)
}

/// Reorders a ranked list so that no more than two videos of the same
/// channel follow each other, moving up the best-ranked video of another
/// channel instead. Items are `(video, channel)`.
pub fn diversify(ranked: Vec<(Uuid, Uuid)>) -> Vec<Uuid> {
    let mut pending: VecDeque<(Uuid, Uuid)> = ranked.into();
    let mut out: Vec<(Uuid, Uuid)> = Vec::with_capacity(pending.len());
    while !pending.is_empty() {
        let repeated = match out.as_slice() {
            [.., a, b] if a.1 == b.1 => Some(b.1),
            _ => None,
        };
        let pick = repeated.and_then(|channel| pending.iter().position(|item| item.1 != channel)).unwrap_or(0);
        out.push(pending.remove(pick).expect("the index is in range"));
    }
    out.into_iter().map(|(video, _)| video).collect()
}

/// Ranks the home feed for `viewer`: video IDs, best first.
async fn rank_home(core: &Core, viewer: &Viewer) -> AppResult<Vec<Uuid>> {
    let candidates: Vec<Candidate> = sqlx::query_as(concat!(
        "WITH candidates AS (
             (SELECT v.id, 3.0::float8 AS boost
              FROM channel_subscriptions s JOIN videos v ON v.channel_id = s.channel_id
              WHERE s.user_id = $1 AND v.published_at > now() - interval '30 days' AND ",
        in_feed!(2, 3),
        "     ORDER BY v.published_at DESC LIMIT 120)
             UNION ALL
             (SELECT v.id, 2.0::float8
              FROM videos v
              WHERE v.owner_id = ANY($2) AND v.published_at > now() - interval '30 days' AND ",
        in_feed!(2, 3),
        "     ORDER BY v.published_at DESC LIMIT 60)
             UNION ALL
             (SELECT v.id, 1.5::float8
              FROM video_reactions r JOIN videos v ON v.id = r.video_id
              WHERE r.user_id = ANY($2) AND r.rating = 2 AND r.created_at > now() - interval '30 days' AND ",
        public_video!(3),
        "     ORDER BY r.created_at DESC LIMIT 60)
             UNION ALL
             (SELECT v.id, 0.0::float8 FROM videos v
              WHERE v.trending_score > 0 AND ",
        public_video!(3),
        "     ORDER BY v.trending_score DESC, v.id DESC LIMIT 150)
             UNION ALL
             (SELECT v.id, 0.5::float8 FROM videos v
              WHERE ",
        public_video!(3),
        "     ORDER BY v.published_at DESC, v.id DESC LIMIT 80)
         )
         SELECT ",
        video_columns!(),
        ", c.boost
         FROM (SELECT id, max(boost) AS boost FROM candidates GROUP BY id) c JOIN videos v ON v.id = c.id
         WHERE ($1::uuid IS NULL OR v.owner_id <> $1)
           AND NOT EXISTS (SELECT 1 FROM watch_history h WHERE h.user_id = $1 AND h.video_id = v.id AND h.completed)
           AND NOT EXISTS (SELECT 1 FROM video_feed_hidden f
                           WHERE f.user_id = $1 AND (f.video_id = v.id OR f.channel_id = v.channel_id))"
    ))
    .bind(viewer.user_id)
    .bind(&viewer.friends)
    .bind(&viewer.blocked)
    .fetch_all(&core.db)
    .await?;

    let current = now();
    let mut scored: Vec<(f64, Uuid, Uuid)> = candidates
        .into_iter()
        .map(|candidate| {
            let video = candidate.video;
            let age_days = (current - video.published_at.unwrap_or(video.created_at)).num_seconds() as f64 / 86_400.0;
            (home_score(candidate.boost, video.trending_score, age_days), video.id, video.channel_id)
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)));
    scored.truncate(HOME_MAX);
    Ok(diversify(scored.into_iter().map(|(_, video, channel)| (video, channel)).collect()))
}

#[derive(Serialize, Deserialize)]
struct HomeCursor {
    /// Names the ranked list kept in Valkey.
    token: String,
    offset: usize,
}

fn home_key(viewer: &Viewer, token: &str) -> String {
    match viewer.user_id {
        Some(user_id) => format!("video:home:{}:{token}", user_id.simple()),
        None => format!("video:home:anon:{token}"),
    }
}

/// One page of the home feed and the cursor of the next (`None`: the end).
pub async fn home_page(core: &Core, viewer: &Viewer, page: &Page) -> AppResult<(Vec<VideoRow>, Option<String>)> {
    let cursor: Option<HomeCursor> = page.decode()?;
    let cached: Option<(String, usize, Vec<Uuid>)> = match cursor {
        Some(cursor) => core
            .cache
            .get_json::<Vec<Uuid>>(&home_key(viewer, &cursor.token))
            .await?
            .map(|ids| (cursor.token, cursor.offset, ids)),
        None => None,
    };
    // A first page, or a cursor whose list expired: rank afresh from the top.
    let (token, offset, ids) = match cached {
        Some(cached) => cached,
        None => {
            let ids = rank_home(core, viewer).await?;
            let token = tokuly_core::ids::random_base62(16);
            core.cache.set_json(&home_key(viewer, &token), &ids, Some(HOME_TTL)).await?;
            (token, 0, ids)
        }
    };
    let end = (offset + page.limit as usize).min(ids.len());
    let slice = ids.get(offset..end).unwrap_or_default();
    // Visibility may have changed since the list was ranked: check again.
    let mut rows: Vec<VideoRow> = sqlx::query_as(concat!(
        "SELECT ",
        video_columns!(),
        " FROM videos v WHERE v.id = ANY($1) AND ",
        in_feed!(2, 3)
    ))
    .bind(slice)
    .bind(&viewer.friends)
    .bind(&viewer.blocked)
    .fetch_all(&core.db)
    .await?;
    rows.sort_by_key(|row| slice.iter().position(|id| *id == row.id).unwrap_or(usize::MAX));
    let next = (end < ids.len()).then(|| tokuly_core::page::encode_cursor(&HomeCursor { token, offset: end }));
    Ok((rows, next))
}

/// Newest videos of the channels `user_id` subscribes to, with the keyset
/// `(published_at, id)` of the last row returned by the previous page.
pub async fn subscriptions_page(
    core: &Core,
    viewer: &Viewer,
    user_id: Uuid,
    before: Option<(Time, Uuid)>,
    limit: i64,
) -> AppResult<Vec<VideoRow>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        video_columns!(),
        " FROM channel_subscriptions s JOIN videos v ON v.channel_id = s.channel_id
          WHERE s.user_id = $1 AND ",
        in_feed!(2, 3),
        "   AND ($4::timestamptz IS NULL OR (v.published_at, v.id) < ($4, $5))
          ORDER BY v.published_at DESC, v.id DESC
          LIMIT $6"
    ))
    .bind(user_id)
    .bind(&viewer.friends)
    .bind(&viewer.blocked)
    .bind(before.map(|(at, _)| at))
    .bind(before.map(|(_, id)| id))
    .bind(limit)
    .fetch_all(&core.db)
    .await?)
}

/// Public videos by trending score. `category` 0 = all.
pub async fn trending_page(
    core: &Core,
    viewer: &Viewer,
    category: i32,
    offset: i64,
    limit: i64,
) -> AppResult<Vec<VideoRow>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        video_columns!(),
        " FROM videos v
          WHERE v.trending_score > 0 AND ($2 = 0 OR v.category = $2) AND ",
        public_video!(1),
        " ORDER BY v.trending_score DESC, v.id DESC
          LIMIT $3 OFFSET $4"
    ))
    .bind(&viewer.blocked)
    .bind(category)
    .bind(limit)
    .bind(offset)
    .fetch_all(&core.db)
    .await?)
}

/// Public videos to watch after `video`: from the same channel, sharing
/// tags, the game or the category — the more in common the higher — topped
/// up with what is trending when there are too few.
pub async fn related(core: &Core, viewer: &Viewer, video: &VideoRow, limit: i64) -> AppResult<Vec<VideoRow>> {
    let mut rows: Vec<VideoRow> = sqlx::query_as(concat!(
        "SELECT ",
        video_columns!(),
        " FROM videos v
          WHERE v.id <> $1 AND ",
        public_video!(6),
        "   AND (v.channel_id = $2 OR v.tags && $3 OR (v.game <> '' AND v.game = $4) OR (v.category <> 0 AND v.category = $5))
            AND NOT EXISTS (SELECT 1 FROM video_feed_hidden f
                            WHERE f.user_id = $8 AND (f.video_id = v.id OR f.channel_id = v.channel_id))
          ORDER BY (v.channel_id = $2)::int * 3
                   + (SELECT count(*) FROM unnest(v.tags) tag WHERE tag = ANY($3)) * 2
                   + (v.game <> '' AND v.game = $4)::int * 2
                   + (v.category <> 0 AND v.category = $5)::int DESC,
                   v.trending_score DESC, v.published_at DESC, v.id DESC
          LIMIT $7"
    ))
    .bind(video.id)
    .bind(video.channel_id)
    .bind(&video.tags)
    .bind(&video.game)
    .bind(video.category)
    .bind(&viewer.blocked)
    .bind(limit)
    .bind(viewer.user_id)
    .fetch_all(&core.db)
    .await?;
    let missing = limit - rows.len() as i64;
    if missing > 0 {
        let mut exclude: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        exclude.push(video.id);
        let more: Vec<VideoRow> = sqlx::query_as(concat!(
            "SELECT ",
            video_columns!(),
            " FROM videos v
              WHERE v.id <> ALL($1) AND ",
            public_video!(2),
            "   AND NOT EXISTS (SELECT 1 FROM video_feed_hidden f
                                WHERE f.user_id = $4 AND (f.video_id = v.id OR f.channel_id = v.channel_id))
              ORDER BY v.trending_score DESC, v.published_at DESC, v.id DESC
              LIMIT $3"
        ))
        .bind(&exclude)
        .bind(&viewer.blocked)
        .bind(missing)
        .bind(viewer.user_id)
        .fetch_all(&core.db)
        .await?;
        rows.extend(more);
    }
    Ok(rows)
}

/// Videos of a list of IDs that `viewer` may watch, in the order given.
pub async fn watchable_in_order(core: &Core, viewer: &Viewer, ids: &[Uuid]) -> AppResult<Vec<VideoRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut rows: Vec<VideoRow> = sqlx::query_as(concat!(
        "SELECT ",
        video_columns!(),
        " FROM videos v WHERE v.id = ANY($1) AND ",
        watchable!(2, 3, 4)
    ))
    .bind(ids)
    .bind(viewer.user_id)
    .bind(&viewer.friends)
    .bind(&viewer.blocked)
    .fetch_all(&core.db)
    .await?;
    rows.sort_by_key(|row| ids.iter().position(|id| *id == row.id).unwrap_or(usize::MAX));
    Ok(rows)
}

/// Recomputes every video's trending score and prunes hourly statistics
/// that no longer matter. Returns how many scores changed.
pub async fn refresh_trending(core: &Core, half_life_hours: f64) -> AppResult<u64> {
    let changed = sqlx::query(
        "WITH scored AS (
             SELECT video_id,
                    sum((views + 4 * likes) * power(0.5::float8, extract(epoch FROM (now() - hour))::float8 / ($1 * 3600.0))) AS score
             FROM video_hourly_stats
             WHERE hour > now() - interval '7 days'
             GROUP BY video_id
         ), target AS (
             SELECT v.id, COALESCE(s.score, 0) AS score
             FROM videos v LEFT JOIN scored s ON s.video_id = v.id
             WHERE s.video_id IS NOT NULL OR v.trending_score > 0
         )
         UPDATE videos v SET trending_score = t.score
         FROM target t
         WHERE v.id = t.id AND v.trending_score IS DISTINCT FROM t.score",
    )
    .bind(half_life_hours)
    .execute(&core.db)
    .await?
    .rows_affected();
    sqlx::query("DELETE FROM video_hourly_stats WHERE hour < now() - interval '8 days'").execute(&core.db).await?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_channel_gets_three_in_a_row() {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        let videos: Vec<Uuid> = (0..7).map(|_| Uuid::now_v7()).collect();
        // Ranked: a a a a b a b
        let ranked = vec![
            (videos[0], a),
            (videos[1], a),
            (videos[2], a),
            (videos[3], a),
            (videos[4], b),
            (videos[5], a),
            (videos[6], b),
        ];
        let out = diversify(ranked);
        // a a [b moved up] a a [b moved up] a
        assert_eq!(out, vec![videos[0], videos[1], videos[4], videos[2], videos[3], videos[6], videos[5]]);
        // When only one channel is left it simply continues.
        let only: Vec<(Uuid, Uuid)> = videos.iter().map(|video| (*video, a)).collect();
        assert_eq!(diversify(only), videos);
        assert!(diversify(Vec::new()).is_empty());
    }

    #[test]
    fn home_scores_reward_source_popularity_and_freshness() {
        let fresh_sub = home_score(3.0, 0.0, 0.0);
        let old_sub = home_score(3.0, 0.0, 30.0);
        let fresh_stranger = home_score(0.5, 0.0, 0.0);
        let viral_stranger = home_score(0.0, 5000.0, 2.0);
        assert!(fresh_sub > old_sub && fresh_sub > fresh_stranger);
        assert!(viral_stranger > old_sub, "something everyone watches beats a month-old subscription video");
        assert!(home_score(0.0, -3.0, -1.0).is_finite());
    }
}
