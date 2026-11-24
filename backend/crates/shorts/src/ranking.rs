//! The for-you ranking: how a short's score is computed, where the
//! candidates of a viewer's feed come from, and how they are ordered for
//! that viewer. `docs/events/shorts.md` explains the same in prose.
//!
//! # 1. The base score (per short, written by the ranking worker)
//!
//! ```text
//! engagement = Σ over the hourly buckets of the last 7 days:
//!                (views + 0.5·loops + 4·likes + 6·comments + 8·shares + 6·saves) · 0.5^(bucket age / half-life)
//! quality    = (Σ watch ratios + 0.5·10) / (plays + 10)          average watch-through, pulled towards 50 %
//!                                                               until ten plays have been reported
//! freshness  = 0.5^(age of the short / (3 · half-life))
//! score      = (1 + ln(1 + engagement)) · (0.4 + 1.2·quality) · (0.25 + 0.75·freshness)
//! ```
//!
//! Engagement is logarithmic (the thousandth view matters less than the
//! tenth), quality can more than triple a score (0.4 to 1.6), and age costs
//! at most three quarters of it. A short nobody has seen yet scores
//! `1 · 1.0 · 1.0`: enough to be shown and collect its first signals.
//! `trending` is the engagement sum on its own.
//!
//! # 2. Candidates (per viewer, when a feed batch is built)
//!
//! Five bounded queries, merged: the newest public shorts, the best scored
//! ones, the newest from followed channels, the newest from friends'
//! channels (friends-only shorts included) and the best scored ones
//! carrying a hashtag of shorts the viewer liked or saved. Never among
//! them: the viewer's own shorts, shorts they marked "not interested",
//! shorts of anyone in a block with them, and (unless the pool is
//! exhausted) shorts they have seen.
//!
//! # 3. The personal rank
//!
//! ```text
//! rank = score · 1.6 (friend's channel) · 1.4 (followed channel)
//!              · (1 + 0.1 · Σ liked-hashtag affinity, at most +0.6)
//!              · 0.35^(shorts of this channel marked "not interested", at most 3)
//!              · 0.7^(hashtags of this short the viewer marked "not interested", at most 3)
//! ```
//!
//! The list is then spread out so that a channel does not appear twice
//! within three consecutive shorts while others are available
//! ([`diversify`]). Everything is deterministic: the same signals give the
//! same feed.
use std::collections::{HashMap, HashSet, VecDeque};

use tokuly_core::prelude::*;
use tokuly_proto::shorts::FeedReason;

use crate::access::{self, Viewer};

pub const VIEW_WEIGHT: f64 = 1.0;
pub const LOOP_WEIGHT: f64 = 0.5;
pub const LIKE_WEIGHT: f64 = 4.0;
pub const COMMENT_WEIGHT: f64 = 6.0;
pub const SHARE_WEIGHT: f64 = 8.0;
pub const SAVE_WEIGHT: f64 = 6.0;
/// Hours of hourly statistics the engagement sum looks back on.
pub const WINDOW_HOURS: i32 = 168;
/// The watch ratio assumed for, and the weight in plays of, a short's
/// prior: a new short is treated as if ten viewers had watched half of it.
pub const PRIOR_RATIO: f64 = 0.5;
pub const PRIOR_PLAYS: f64 = 10.0;

pub const FRIEND_BOOST: f64 = 1.6;
pub const FOLLOWING_BOOST: f64 = 1.4;
/// Gain per unit of hashtag affinity, and the most hashtags can add.
pub const HASHTAG_GAIN: f64 = 0.1;
pub const HASHTAG_GAIN_CAP: f64 = 0.6;
/// A liked hashtag counts at most this often.
pub const AFFINITY_CAP: f64 = 3.0;
pub const HIDDEN_CHANNEL_FACTOR: f64 = 0.35;
pub const HIDDEN_HASHTAG_FACTOR: f64 = 0.7;
/// "Not interested" marks counted per channel, and hashtags per short.
pub const PENALTY_CAP: i32 = 3;
/// Decayed engagement from which a short is called trending in the feed.
pub const TRENDING_REASON_MIN: f64 = 5.0;

/// Hashtags of liked and saved shorts that shape a viewer's feed.
const TASTE_HASHTAGS: i64 = 20;
/// Days a like, a save or a "not interested" mark shapes the feed.
const TASTE_DAYS: i32 = 90;
/// "Not interested" marks considered.
const TASTE_HIDDEN: i64 = 200;

// ───────────────────────────── the base score ─────────────────────────────

/// The weighted engagement of one hourly bucket, decayed by its age.
/// A bucket whose sum is negative (more likes taken back than given)
/// counts as nothing.
pub fn decayed_engagement(
    views: i64,
    loops: i64,
    likes: i64,
    comments: i64,
    shares: i64,
    saves: i64,
    age_hours: f64,
    half_life_hours: f64,
) -> f64 {
    let points = VIEW_WEIGHT * views as f64
        + LOOP_WEIGHT * loops as f64
        + LIKE_WEIGHT * likes as f64
        + COMMENT_WEIGHT * comments as f64
        + SHARE_WEIGHT * shares as f64
        + SAVE_WEIGHT * saves as f64;
    points.max(0.0) * 0.5f64.powf(age_hours.max(0.0) / half_life_hours)
}

/// Average watch-through ratio, smoothed towards [`PRIOR_RATIO`] while few
/// plays have been reported.
pub fn quality(watch_ratio_sum: f64, watch_samples: i64) -> f64 {
    ((watch_ratio_sum + PRIOR_RATIO * PRIOR_PLAYS) / (watch_samples.max(0) as f64 + PRIOR_PLAYS)).clamp(0.0, 1.0)
}

/// How new a short is, from 1 (just published) towards 0.
pub fn freshness(age_hours: f64, half_life_hours: f64) -> f64 {
    0.5f64.powf(age_hours.max(0.0) / (3.0 * half_life_hours))
}

/// The for-you base score of a short.
pub fn base_score(engagement: f64, quality: f64, freshness: f64) -> f64 {
    (1.0 + engagement.max(0.0).ln_1p()) * (0.4 + 1.2 * quality) * (0.25 + 0.75 * freshness)
}

/// Recomputes `score` and `trending` of every short whose score can still
/// move: shorts with statistics in the window, shorts published during the
/// last 30 days, shorts that were trending at the last run, and shorts that
/// were never scored. Also prunes hourly statistics that fell out of the
/// window. Returns how many shorts were scored.
///
/// The SQL below is the formula of the module documentation; the
/// integration test `scores_follow_the_documented_formula` holds it against
/// the Rust functions above.
pub async fn refresh_scores(core: &Core, half_life_hours: f64) -> AppResult<u64> {
    // One refresh at a time, for the same reason as the hashtags' trending refresh.
    let mut tx = core.db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('shorts:scores', 0))").execute(&mut *tx).await?;
    let scored = sqlx::query(
        "WITH engagement AS (
             SELECT h.short_id,
                    sum(GREATEST(h.views + 0.5 * h.loops + 4 * h.likes + 6 * h.comments + 8 * h.shares + 6 * h.saves, 0)
                        * power(0.5::float8, GREATEST(extract(epoch FROM now() - h.hour) / 3600.0, 0) / $1)) AS points
             FROM short_hourly_stats h
             WHERE h.hour > now() - make_interval(hours => $2)
             GROUP BY h.short_id
         ), working AS (
             SELECT s.id FROM shorts s
             WHERE s.status = 2
               AND (s.scored_at IS NULL OR s.trending > 0 OR s.published_at > now() - interval '30 days'
                    OR s.id IN (SELECT short_id FROM engagement))
         )
         UPDATE shorts s
         SET trending = COALESCE(e.points, 0),
             score = (1 + ln(1 + COALESCE(e.points, 0)))
                   * (0.4 + 1.2 * LEAST(GREATEST((s.watch_ratio_sum + 5.0) / (s.watch_samples + 10.0), 0), 1))
                   * (0.25 + 0.75 * power(0.5::float8,
                          GREATEST(extract(epoch FROM now() - COALESCE(s.published_at, s.created_at)) / 3600.0, 0) / (3 * $1))),
             scored_at = now()
         FROM working w LEFT JOIN engagement e ON e.short_id = w.id
         WHERE s.id = w.id",
    )
    .bind(half_life_hours)
    .bind(WINDOW_HOURS)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    sqlx::query("DELETE FROM short_hourly_stats WHERE hour < now() - make_interval(hours => $1)")
        .bind(WINDOW_HOURS + 1)
        .execute(&core.db)
        .await?;
    Ok(scored)
}

// ───────────────────────────── the viewer's taste ─────────────────────────────

/// What is known about a viewer that shapes their feed.
#[derive(Debug, Clone, Default)]
pub struct Taste {
    /// Hashtags of shorts the viewer liked or saved recently, with how many
    /// such shorts carried each.
    pub liked_hashtags: HashMap<String, f64>,
    /// "Not interested" marks per channel and per hashtag.
    pub hidden_channels: HashMap<Uuid, i32>,
    pub hidden_hashtags: HashMap<String, i32>,
    /// Channels the viewer follows.
    pub followed: HashSet<Uuid>,
    /// The viewer's friends (owners of channels).
    pub friends: HashSet<Uuid>,
}

impl Taste {
    /// Loads the taste of a signed-in viewer; an anonymous one has none.
    pub async fn load(core: &Core, viewer: &Viewer) -> AppResult<Self> {
        let Some(user_id) = viewer.user_id else { return Ok(Self::default()) };
        let liked: Vec<(String, i64)> = sqlx::query_as(
            "SELECT tag, count(*) FROM (
                 SELECT unnest(s.hashtags) AS tag FROM short_likes l JOIN shorts s ON s.id = l.short_id
                 WHERE l.user_id = $1 AND l.liked AND l.created_at > now() - make_interval(days => $2)
                 UNION ALL
                 SELECT unnest(s.hashtags) FROM short_saves v JOIN shorts s ON s.id = v.short_id
                 WHERE v.user_id = $1 AND v.created_at > now() - make_interval(days => $2)
             ) tags
             GROUP BY tag ORDER BY count(*) DESC, tag LIMIT $3",
        )
        .bind(user_id)
        .bind(TASTE_DAYS)
        .bind(TASTE_HASHTAGS)
        .fetch_all(&core.db)
        .await?;
        let hidden: Vec<(Uuid, Vec<String>)> = sqlx::query_as(
            "SELECT channel_id, hashtags FROM short_hidden
             WHERE user_id = $1 AND created_at > now() - make_interval(days => $2)
             ORDER BY created_at DESC LIMIT $3",
        )
        .bind(user_id)
        .bind(TASTE_DAYS)
        .bind(TASTE_HIDDEN)
        .fetch_all(&core.db)
        .await?;
        let followed: Vec<Uuid> = sqlx::query_scalar("SELECT channel_id FROM short_follows WHERE user_id = $1")
            .bind(user_id)
            .fetch_all(&core.db)
            .await?;
        let mut taste = Self {
            liked_hashtags: liked.into_iter().map(|(tag, count)| (tag, count as f64)).collect(),
            followed: followed.into_iter().collect(),
            friends: viewer.friends.iter().copied().collect(),
            ..Default::default()
        };
        for (channel_id, hashtags) in hidden {
            *taste.hidden_channels.entry(channel_id).or_default() += 1;
            for tag in hashtags {
                *taste.hidden_hashtags.entry(tag).or_default() += 1;
            }
        }
        Ok(taste)
    }
}

// ───────────────────────────── candidates ─────────────────────────────

/// A short that might go into a viewer's feed.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Candidate {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub owner_id: Uuid,
    pub hashtags: Vec<String>,
    pub score: f64,
    pub trending: f64,
    pub published_at: Option<Time>,
}

/// One candidate query: shorts the viewer's feeds may carry (the viewer as
/// `$1`–`$6`, see `tokuly_video::access`), without their own, the ones
/// they marked "not interested" and the ones in `$7` (seen), narrowed by
/// `$filter` and cut to `$8` in `$order`.
macro_rules! candidate_query {
    ($filter:literal, $order:literal) => {
        concat!(
            "SELECT s.id, s.channel_id, s.owner_id, s.hashtags, s.score, s.trending, s.published_at FROM shorts s WHERE ",
            $filter,
            " AND ",
            access::in_feed!(),
            " AND ($1::uuid IS NULL OR s.owner_id <> $1)
              AND NOT (s.id = ANY($7))
              AND NOT EXISTS (SELECT 1 FROM short_hidden x WHERE x.user_id = $1 AND x.short_id = s.id)
              ORDER BY ",
            $order,
            " LIMIT $8"
        )
    };
}

/// Gathers the candidates of a viewer's for-you feed from the five sources
/// described in the module documentation, at most `per_source` from each,
/// leaving out the shorts in `seen`.
pub async fn candidates(
    core: &Core,
    viewer: &Viewer,
    taste: &Taste,
    seen: &[Uuid],
    per_source: i64,
) -> AppResult<Vec<Candidate>> {
    let mut found: Vec<Candidate> = Vec::new();
    let recent: &str = candidate_query!("s.listed", "s.published_at DESC, s.id DESC");
    let top: &str = candidate_query!("s.listed", "s.score DESC, s.id DESC");
    for sql in [recent, top] {
        let rows: Vec<Candidate> = viewer.query_as(sql).bind(seen).bind(per_source).fetch_all(&core.db).await?;
        found.extend(rows);
    }
    if !taste.followed.is_empty() {
        let followed: Vec<Uuid> = taste.followed.iter().copied().collect();
        let rows: Vec<Candidate> = viewer
            .query_as(candidate_query!("s.channel_id = ANY($9)", "s.published_at DESC, s.id DESC"))
            .bind(seen)
            .bind(per_source)
            .bind(&followed)
            .fetch_all(&core.db)
            .await?;
        found.extend(rows);
    }
    if !viewer.friends.is_empty() {
        let rows: Vec<Candidate> = viewer
            .query_as(candidate_query!("s.owner_id = ANY($2)", "s.published_at DESC, s.id DESC"))
            .bind(seen)
            .bind(per_source)
            .fetch_all(&core.db)
            .await?;
        found.extend(rows);
    }
    if !taste.liked_hashtags.is_empty() {
        let hashtags: Vec<String> = taste.liked_hashtags.keys().cloned().collect();
        let rows: Vec<Candidate> = viewer
            .query_as(candidate_query!("s.listed AND s.hashtags && $9", "s.score DESC, s.id DESC"))
            .bind(seen)
            .bind(per_source)
            .bind(&hashtags)
            .fetch_all(&core.db)
            .await?;
        found.extend(rows);
    }
    let mut known = HashSet::new();
    found.retain(|candidate| known.insert(candidate.id));
    Ok(found)
}

// ───────────────────────────── the personal rank ─────────────────────────────

/// A candidate with its place in one viewer's feed.
#[derive(Debug, Clone, PartialEq)]
pub struct Ranked {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub owner_id: Uuid,
    pub rank: f64,
    pub reason: FeedReason,
    /// For [`FeedReason::FEED_REASON_HASHTAG`]: the hashtag that matched.
    pub hashtag: Option<String>,
}

/// Ranks one candidate for a viewer (see the module documentation).
pub fn personal_rank(candidate: &Candidate, taste: &Taste) -> Ranked {
    let friend = taste.friends.contains(&candidate.owner_id);
    let following = taste.followed.contains(&candidate.channel_id);
    // The liked hashtag this short shares most strongly, and all of them together.
    let mut best: Option<(&String, f64)> = None;
    let mut affinity = 0.0;
    for tag in &candidate.hashtags {
        if let Some(count) = taste.liked_hashtags.get(tag) {
            let weight = count.min(AFFINITY_CAP);
            affinity += weight;
            if best.is_none_or(|(_, strongest)| weight > strongest) {
                best = Some((tag, weight));
            }
        }
    }
    let hidden_channel = taste.hidden_channels.get(&candidate.channel_id).copied().unwrap_or(0).min(PENALTY_CAP);
    let hidden_hashtags = (candidate.hashtags.iter().filter(|tag| taste.hidden_hashtags.contains_key(*tag)).count()
        as i32)
        .min(PENALTY_CAP);

    let mut rank = candidate.score;
    if friend {
        rank *= FRIEND_BOOST;
    }
    if following {
        rank *= FOLLOWING_BOOST;
    }
    rank *= 1.0 + (HASHTAG_GAIN * affinity).min(HASHTAG_GAIN_CAP);
    rank *= HIDDEN_CHANNEL_FACTOR.powi(hidden_channel) * HIDDEN_HASHTAG_FACTOR.powi(hidden_hashtags);

    let reason = if friend {
        FeedReason::FEED_REASON_FRIEND
    } else if following {
        FeedReason::FEED_REASON_FOLLOWING
    } else if best.is_some() {
        FeedReason::FEED_REASON_HASHTAG
    } else if candidate.trending >= TRENDING_REASON_MIN {
        FeedReason::FEED_REASON_TRENDING
    } else {
        FeedReason::FEED_REASON_NEW
    };
    Ranked {
        id: candidate.id,
        channel_id: candidate.channel_id,
        owner_id: candidate.owner_id,
        rank,
        hashtag: best.filter(|_| reason == FeedReason::FEED_REASON_HASHTAG).map(|(tag, _)| tag.clone()),
        reason,
    }
}

/// Orders candidates for a viewer: best rank first (newer first among
/// equals), then spread out by channel.
pub fn rank(candidates: &[Candidate], taste: &Taste) -> Vec<Ranked> {
    let mut ranked: Vec<Ranked> = candidates.iter().map(|candidate| personal_rank(candidate, taste)).collect();
    // UUIDv7 IDs grow with time: the larger ID is the newer short.
    ranked.sort_by(|a, b| b.rank.total_cmp(&a.rank).then_with(|| b.id.cmp(&a.id)));
    diversify(ranked)
}

/// Spreads a ranked list out so that no channel appears twice within three
/// consecutive places while shorts of other channels are left. Items keep
/// their relative order otherwise; one that has to wait takes the first
/// place where it fits.
pub fn diversify(ranked: Vec<Ranked>) -> Vec<Ranked> {
    let mut out: Vec<Ranked> = Vec::with_capacity(ranked.len());
    let mut waiting: VecDeque<Ranked> = VecDeque::new();
    let mut incoming = ranked.into_iter();
    loop {
        let recent: Vec<Uuid> = out.iter().rev().take(2).map(|item| item.channel_id).collect();
        if let Some(position) = waiting.iter().position(|item| !recent.contains(&item.channel_id)) {
            out.extend(waiting.remove(position));
            continue;
        }
        match incoming.next() {
            Some(item) if recent.contains(&item.channel_id) => waiting.push_back(item),
            Some(item) => out.push(item),
            None => break,
        }
    }
    // Only shorts of the channels just shown are left: nothing to spread them with.
    out.extend(waiting);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(score: f64) -> Candidate {
        Candidate {
            id: Uuid::now_v7(),
            channel_id: Uuid::now_v7(),
            owner_id: Uuid::now_v7(),
            hashtags: Vec::new(),
            score,
            trending: 0.0,
            published_at: None,
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn an_unseen_short_scores_one_and_engagement_counts_logarithmically() {
        assert!(close(base_score(0.0, quality(0.0, 0), freshness(0.0, 24.0)), 1.0));
        let ten = base_score(10.0, 0.5, 1.0);
        let thousand = base_score(1000.0, 0.5, 1.0);
        assert!(ten > 1.0 && thousand > ten);
        assert!(thousand - ten < 10.0 * (ten - 1.0), "the thousandth point matters less than the tenth");
        assert!(close(base_score(-5.0, 0.5, 1.0), 1.0), "negative engagement is no engagement");
    }

    #[test]
    fn quality_is_smoothed_until_enough_plays_are_in() {
        assert!(close(quality(0.0, 0), 0.5));
        // One viewer who watched it all barely moves the prior.
        assert!(close(quality(1.0, 1), 6.0 / 11.0));
        // A thousand who did make it certain.
        assert!(quality(1000.0, 1000) > 0.99);
        assert!(quality(0.0, 1000) < 0.01);
        // Quality moves the score between 0.4 and 1.6 times.
        assert!(close(base_score(0.0, 0.0, 1.0), 0.4) && close(base_score(0.0, 1.0, 1.0), 1.6));
    }

    #[test]
    fn signals_and_shorts_fade_with_time() {
        let fresh = decayed_engagement(10, 0, 0, 0, 0, 0, 0.0, 24.0);
        assert!(close(fresh, 10.0));
        assert!(close(decayed_engagement(10, 0, 0, 0, 0, 0, 24.0, 24.0), 5.0));
        assert!(close(decayed_engagement(10, 0, 0, 0, 0, 0, 48.0, 24.0), 2.5));
        // A share is worth eight views, a like four.
        assert!(close(decayed_engagement(0, 2, 1, 1, 1, 1, 0.0, 24.0), 1.0 + 4.0 + 6.0 + 8.0 + 6.0));
        assert!(close(decayed_engagement(1, 0, -3, 0, 0, 0, 0.0, 24.0), 0.0));
        // A short's freshness halves every three half-lives and costs at most 75 %.
        assert!(close(freshness(72.0, 24.0), 0.5));
        assert!(close(base_score(0.0, 0.5, 0.0), 0.25));
    }

    #[test]
    fn friends_follows_and_liked_hashtags_lift_a_short() {
        let mut plain = candidate(2.0);
        plain.hashtags = vec!["dance".into(), "cat".into()];
        let taste = Taste::default();
        let neutral = personal_rank(&plain, &taste);
        assert!(close(neutral.rank, 2.0));
        assert_eq!(neutral.reason, FeedReason::FEED_REASON_NEW);

        let friends = Taste { friends: HashSet::from([plain.owner_id]), ..Default::default() };
        let lifted = personal_rank(&plain, &friends);
        assert!(close(lifted.rank, 2.0 * FRIEND_BOOST));
        assert_eq!(lifted.reason, FeedReason::FEED_REASON_FRIEND);

        let follows = Taste { followed: HashSet::from([plain.channel_id]), ..Default::default() };
        assert_eq!(personal_rank(&plain, &follows).reason, FeedReason::FEED_REASON_FOLLOWING);
        assert!(close(personal_rank(&plain, &follows).rank, 2.0 * FOLLOWING_BOOST));

        let likes = Taste {
            liked_hashtags: HashMap::from([("dance".to_owned(), 2.0), ("cat".to_owned(), 9.0)]),
            ..Default::default()
        };
        let matched = personal_rank(&plain, &likes);
        // 2 + min(9, 3) = 5 units of affinity: +50 %.
        assert!(close(matched.rank, 2.0 * 1.5));
        assert_eq!((matched.reason, matched.hashtag.as_deref()), (FeedReason::FEED_REASON_HASHTAG, Some("cat")));

        // Hashtags never add more than 60 %.
        plain.hashtags = (0..10).map(|i| format!("t{i}")).collect();
        let obsessed = Taste {
            liked_hashtags: plain.hashtags.iter().map(|tag| (tag.clone(), 3.0)).collect(),
            ..Default::default()
        };
        assert!(close(personal_rank(&plain, &obsessed).rank, 2.0 * 1.6));
    }

    #[test]
    fn not_interested_pushes_a_channel_and_its_hashtags_down() {
        let mut short = candidate(2.0);
        short.hashtags = vec!["prank".into(), "cat".into()];
        let once = Taste {
            hidden_channels: HashMap::from([(short.channel_id, 1)]),
            hidden_hashtags: HashMap::from([("prank".to_owned(), 1)]),
            ..Default::default()
        };
        assert!(close(personal_rank(&short, &once).rank, 2.0 * HIDDEN_CHANNEL_FACTOR * HIDDEN_HASHTAG_FACTOR));
        // Another channel carrying the same hashtag is pushed down less.
        let mut other = candidate(2.0);
        other.hashtags = vec!["prank".into()];
        assert!(close(personal_rank(&other, &once).rank, 2.0 * HIDDEN_HASHTAG_FACTOR));
        // Penalties stop growing after three marks.
        let often = Taste { hidden_channels: HashMap::from([(short.channel_id, 9)]), ..Default::default() };
        assert!(close(personal_rank(&short, &often).rank, 2.0 * HIDDEN_CHANNEL_FACTOR.powi(3)));
    }

    #[test]
    fn trending_shorts_are_called_trending() {
        let mut hot = candidate(5.0);
        hot.trending = TRENDING_REASON_MIN;
        assert_eq!(personal_rank(&hot, &Taste::default()).reason, FeedReason::FEED_REASON_TRENDING);
    }

    #[test]
    fn ranking_orders_by_rank_and_breaks_ties_by_age() {
        let older = candidate(1.0);
        let newer = candidate(1.0);
        let best = candidate(3.0);
        let order: Vec<Uuid> =
            rank(&[older.clone(), newer.clone(), best.clone()], &Taste::default()).iter().map(|r| r.id).collect();
        assert_eq!(order, [best.id, newer.id, older.id]);
    }

    fn of_channel(channel_id: Uuid, rank: f64) -> Ranked {
        Ranked {
            id: Uuid::now_v7(),
            channel_id,
            owner_id: Uuid::now_v7(),
            rank,
            reason: FeedReason::FEED_REASON_NEW,
            hashtag: None,
        }
    }

    #[test]
    fn a_channel_does_not_fill_three_places_in_a_row() {
        let (a, b, c) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let input = vec![
            of_channel(a, 9.0),
            of_channel(a, 8.0),
            of_channel(a, 7.0),
            of_channel(b, 6.0),
            of_channel(c, 5.0),
            of_channel(b, 4.0),
        ];
        let ids: Vec<Uuid> = input.iter().map(|item| item.id).collect();
        let spread = diversify(input);
        let channels: Vec<Uuid> = spread.iter().map(|item| item.channel_id).collect();
        assert_eq!(channels, [a, b, c, a, b, a]);
        // Nothing is lost and the best short still leads.
        assert_eq!(spread.len(), 6);
        assert_eq!(spread[0].id, ids[0]);
        assert_eq!(spread[3].id, ids[1], "the waiting short takes the first place where it fits");
    }

    #[test]
    fn one_channel_alone_keeps_its_order() {
        let a = Uuid::now_v7();
        let input = vec![of_channel(a, 3.0), of_channel(a, 2.0), of_channel(a, 1.0)];
        let ids: Vec<Uuid> = input.iter().map(|item| item.id).collect();
        assert_eq!(diversify(input).iter().map(|item| item.id).collect::<Vec<_>>(), ids);
        assert!(diversify(Vec::new()).is_empty());
    }
}
