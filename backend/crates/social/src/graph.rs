//! The friend graph as the rest of the backend sees it.
//!
//! Other domains call these functions to decide who may see or do what:
//! geo fans locations out to [`friend_ids`], chat asks [`allows`] before
//! opening a DM, feeds and search hide [`blocked_ids_either`], and anything
//! two friends do together is reported through [`record_interaction`], which
//! feeds the best-friend ranking.
//!
//! Yes/no questions that gate access ([`are_friends`], [`is_blocked_either`],
//! [`allows`]) always read PostgreSQL, so an unfriend or a block takes effect
//! immediately. Only the two list functions are cached (see [`crate::idcache`]).
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use tokuly_core::prelude::*;
use tokuly_proto::identity::{Audience, Relationship};

use crate::{
    idcache,
    ranking::{self, Bucket},
};

/// Something two friends did, reported by the domain where it happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InteractionKind {
    /// A chat message; weight = number of messages.
    Message,
    /// A stamp sent in chat; weight = number of stamps.
    Stamp,
    /// A bump; weight = number of bumps.
    Bump,
    /// One opened the other's pin on the map; weight = number of views.
    MapView,
    /// A map reaction ("supersend"); weight = number of reactions.
    MapReaction,
    /// A voice or video call; weight = minutes on the call.
    Call,
    /// Physically together; weight = minutes together. Report it every few
    /// minutes while a gathering lasts rather than once at the end, so nights
    /// spent together are recognized.
    TimeTogether,
}

impl InteractionKind {
    fn bucket(self) -> Bucket {
        match self {
            Self::Message => Bucket::Message,
            Self::Stamp => Bucket::Stamp,
            Self::Bump => Bucket::Bump,
            Self::MapView => Bucket::MapView,
            Self::MapReaction => Bucket::MapReaction,
            Self::Call => Bucket::Call,
            Self::TimeTogether => Bucket::TimeTogether,
        }
    }
}

/// Two users in canonical order (`lo < hi`): the key of everything stored
/// once per pair rather than once per direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Pair {
    pub lo: Uuid,
    pub hi: Uuid,
}

impl Pair {
    /// Orders two distinct users. (`Uuid` and PostgreSQL's `uuid` both compare
    /// byte-wise, so Rust and SQL agree on which one is `lo`.)
    pub(crate) fn of(a: Uuid, b: Uuid) -> Self {
        if a < b { Self { lo: a, hi: b } } else { Self { lo: b, hi: a } }
    }
}

/// Serializes concurrent changes to one pair's relationship (requests,
/// accepts, removals, blocks) for the rest of the transaction. Without it two
/// crossed friend requests could both be stored instead of becoming a
/// friendship.
pub(crate) async fn lock_pair(tx: &mut Transaction<'_, Postgres>, a: Uuid, b: Uuid) -> AppResult<()> {
    let pair = Pair::of(a, b);
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("social:pair:{}:{}", pair.lo.simple(), pair.hi.simple()))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// IDs of a user's friends. Hot path: served from Valkey, loaded from the
/// database on a miss.
pub async fn friend_ids(core: &Core, user_id: Uuid) -> AppResult<Vec<Uuid>> {
    idcache::FRIENDS
        .get_or_load(core, user_id, || async {
            Ok(sqlx::query_scalar("SELECT friend_id FROM friendships WHERE user_id = $1").bind(user_id).fetch_all(&core.db).await?)
        })
        .await
}

/// How many friends a user has.
pub(crate) async fn friend_count(core: &Core, user_id: Uuid) -> AppResult<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM friendships WHERE user_id = $1").bind(user_id).fetch_one(&core.db).await?)
}

/// How many friends of `owner` a viewer gets to see: active accounts other
/// than `hidden` (the viewer's [`blocked_ids_either`]). The count a viewer
/// is shown must match the list they are shown, or the difference tells
/// them who blocked them.
pub(crate) async fn visible_friend_count(core: &Core, owner: Uuid, hidden: &[Uuid]) -> AppResult<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM friendships f JOIN users u ON u.id = f.friend_id
         WHERE f.user_id = $1 AND u.deleted_at IS NULL AND NOT (f.friend_id = ANY($2))",
    )
    .bind(owner)
    .bind(hidden)
    .fetch_one(&core.db)
    .await?)
}

pub async fn are_friends(core: &Core, a: Uuid, b: Uuid) -> AppResult<bool> {
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM friendships WHERE user_id = $1 AND friend_id = $2)")
        .bind(a)
        .bind(b)
        .fetch_one(&core.db)
        .await?)
}

/// Whether `a` blocked `b` or `b` blocked `a`.
pub async fn is_blocked_either(core: &Core, a: Uuid, b: Uuid) -> AppResult<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM user_blocks
                       WHERE (blocker_id = $1 AND blocked_id = $2) OR (blocker_id = $2 AND blocked_id = $1))",
    )
    .bind(a)
    .bind(b)
    .fetch_one(&core.db)
    .await?)
}

/// Everyone hidden from `user_id`: the users they blocked and the users who
/// blocked them. Cached like [`friend_ids`].
pub async fn blocked_ids_either(core: &Core, user_id: Uuid) -> AppResult<Vec<Uuid>> {
    idcache::HIDDEN
        .get_or_load(core, user_id, || async {
            Ok(sqlx::query_scalar(
                "SELECT blocked_id FROM user_blocks WHERE blocker_id = $1
                 UNION
                 SELECT blocker_id FROM user_blocks WHERE blocked_id = $1",
            )
            .bind(user_id)
            .fetch_all(&core.db)
            .await?)
        })
        .await
}

/// The users `user_id` blocked themselves: the part of
/// [`blocked_ids_either`] that is theirs to decide (the rest is who blocked
/// them, which anybody can add to). Read from PostgreSQL, not cached.
pub async fn blocked_by(core: &Core, user_id: Uuid) -> AppResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar("SELECT blocked_id FROM user_blocks WHERE blocker_id = $1")
        .bind(user_id)
        .fetch_all(&core.db)
        .await?)
}

/// What `viewer` is to `other`. Being blocked by `other` is deliberately not
/// a relationship of its own: blocks are silent, so it reads as `NONE`.
pub async fn relationship(core: &Core, viewer: Uuid, other: Uuid) -> AppResult<Relationship> {
    Ok(relationships(core, viewer, &[other]).await?.remove(&other).unwrap_or(Relationship::RELATIONSHIP_NONE))
}

/// [`relationship`] for many users at once. Every requested user is in the map.
pub(crate) async fn relationships(core: &Core, viewer: Uuid, others: &[Uuid]) -> AppResult<HashMap<Uuid, Relationship>> {
    // One row per fact; several facts about the same user are resolved below
    // by precedence (a declined incoming request is not a fact).
    const BLOCKED: i32 = 1;
    const FRIENDS: i32 = 2;
    const SENT: i32 = 3;
    const RECEIVED: i32 = 4;
    let facts: Vec<(Uuid, i32)> = sqlx::query_as(
        "SELECT blocked_id, 1 FROM user_blocks WHERE blocker_id = $1 AND blocked_id = ANY($2)
         UNION ALL
         SELECT friend_id, 2 FROM friendships WHERE user_id = $1 AND friend_id = ANY($2)
         UNION ALL
         SELECT to_user_id, 3 FROM friend_requests WHERE from_user_id = $1 AND to_user_id = ANY($2)
         UNION ALL
         SELECT from_user_id, 4 FROM friend_requests WHERE to_user_id = $1 AND from_user_id = ANY($2) AND declined_at IS NULL",
    )
    .bind(viewer)
    .bind(others)
    .fetch_all(&core.db)
    .await?;
    let mut strongest: HashMap<Uuid, i32> = HashMap::new();
    for (other, fact) in facts {
        strongest.entry(other).and_modify(|current| *current = (*current).min(fact)).or_insert(fact);
    }
    Ok(others
        .iter()
        .map(|other| {
            let relationship = if *other == viewer {
                Relationship::RELATIONSHIP_SELF
            } else {
                match strongest.get(other) {
                    Some(&BLOCKED) => Relationship::RELATIONSHIP_BLOCKED,
                    Some(&FRIENDS) => Relationship::RELATIONSHIP_FRIENDS,
                    Some(&SENT) => Relationship::RELATIONSHIP_REQUEST_SENT,
                    Some(&RECEIVED) => Relationship::RELATIONSHIP_REQUEST_RECEIVED,
                    _ => Relationship::RELATIONSHIP_NONE,
                }
            };
            (*other, relationship)
        })
        .collect())
}

/// Whether `viewer` falls into `audience` as chosen by `owner` in a privacy
/// setting ("who may message me", "who may see my friend list"...).
///
/// The owner always passes; anyone in a block with the owner never does;
/// anonymous viewers only pass `EVERYONE`. An unset audience is treated as
/// friends-only, the cautious reading of a setting that was never stored.
pub async fn allows(core: &Core, owner: Uuid, viewer: Option<Uuid>, audience: Audience) -> AppResult<bool> {
    let Some(viewer) = viewer else {
        return Ok(audience == Audience::AUDIENCE_EVERYONE);
    };
    if viewer == owner {
        return Ok(true);
    }
    if is_blocked_either(core, owner, viewer).await? {
        return Ok(false);
    }
    match audience {
        Audience::AUDIENCE_EVERYONE => Ok(true),
        Audience::AUDIENCE_NOBODY => Ok(false),
        Audience::AUDIENCE_FRIENDS | Audience::AUDIENCE_UNSPECIFIED => are_friends(core, owner, viewer).await,
        Audience::AUDIENCE_FRIENDS_OF_FRIENDS => Ok(are_friends(core, owner, viewer).await? || have_mutual_friend(core, owner, viewer).await?),
    }
}

async fn have_mutual_friend(core: &Core, a: Uuid, b: Uuid) -> AppResult<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM friendships x JOIN friendships y ON y.friend_id = x.friend_id
                       WHERE x.user_id = $1 AND y.user_id = $2)",
    )
    .bind(a)
    .bind(b)
    .fetch_one(&core.db)
    .await?)
}

/// How many friends `a` and `b` have in common.
pub async fn mutual_friend_count(core: &Core, a: Uuid, b: Uuid) -> AppResult<i32> {
    Ok(mutual_friend_counts(core, a, &[b]).await?.get(&b).copied().unwrap_or(0))
}

/// [`mutual_friend_count`] between `viewer` and many users; users without
/// mutual friends are absent from the map.
pub(crate) async fn mutual_friend_counts(core: &Core, viewer: Uuid, others: &[Uuid]) -> AppResult<HashMap<Uuid, i32>> {
    if others.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT theirs.user_id, count(*)
         FROM friendships mine JOIN friendships theirs ON theirs.friend_id = mine.friend_id
         WHERE mine.user_id = $1 AND theirs.user_id = ANY($2) AND theirs.user_id <> $1
         GROUP BY theirs.user_id",
    )
    .bind(viewer)
    .bind(others)
    .fetch_all(&core.db)
    .await?;
    Ok(rows.into_iter().map(|(id, count)| (id, count as i32)).collect())
}

/// Up to `limit` friends `a` and `b` share (a's oldest friendships first).
pub(crate) async fn mutual_friend_ids(core: &Core, a: Uuid, b: Uuid, limit: i64) -> AppResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar(
        "SELECT mine.friend_id
         FROM friendships mine JOIN friendships theirs ON theirs.friend_id = mine.friend_id AND theirs.user_id = $2
         WHERE mine.user_id = $1
         ORDER BY mine.created_at, mine.friend_id
         LIMIT $3",
    )
    .bind(a)
    .bind(b)
    .bind(limit)
    .fetch_all(&core.db)
    .await?)
}

/// Largest weight accepted for one report (a full day in minutes).
const MAX_WEIGHT: i32 = 1440;

/// Records that `from` did something with or towards `to`. Feeds the
/// best-friend ranking, the weekly streak and the time-together counters.
///
/// `weight` is how much of it happened: a count for messages, stamps, bumps,
/// map views and map reactions (usually 1), minutes for calls and time
/// together. Interactions between users who are not friends are ignored, so
/// callers need not check first.
pub async fn record_interaction(core: &Core, from: Uuid, to: Uuid, kind: InteractionKind, weight: i32) -> AppResult<()> {
    if from == to || weight <= 0 {
        return Ok(());
    }
    let at = Utc::now();
    let weight = weight.min(MAX_WEIGHT);
    let pair = Pair::of(from, to);
    let (lo_weight, hi_weight) = if from == pair.lo { (weight as i64, 0_i64) } else { (0, weight as i64) };
    let day = at.date_naive();
    let seconds_together = if kind == InteractionKind::TimeTogether { weight as i64 * 60 } else { 0 };

    // One statement: touch the pair (only if the two are friends), advance
    // its streak when this is the first interaction of a new week, and add
    // the weight to the day bucket.
    let recorded = sqlx::query(
        "WITH pair AS (
             INSERT INTO friend_pairs (user_lo, user_hi, last_interaction_at, dirty, streak_weeks, streak_week, time_together_seconds)
             SELECT $1, $2, $3, true, 1, $4, $5
             WHERE EXISTS (SELECT 1 FROM friendships WHERE user_id = $1 AND friend_id = $2)
             ON CONFLICT (user_lo, user_hi) DO UPDATE SET
                 last_interaction_at = GREATEST(friend_pairs.last_interaction_at, EXCLUDED.last_interaction_at),
                 dirty = true,
                 streak_weeks = CASE
                     WHEN friend_pairs.streak_week >= EXCLUDED.streak_week THEN friend_pairs.streak_weeks
                     WHEN friend_pairs.streak_week = EXCLUDED.streak_week - 7 THEN friend_pairs.streak_weeks + 1
                     ELSE 1 END,
                 streak_week = GREATEST(friend_pairs.streak_week, EXCLUDED.streak_week),
                 time_together_seconds = friend_pairs.time_together_seconds + EXCLUDED.time_together_seconds
             RETURNING user_lo
         )
         INSERT INTO friend_interaction_days (user_lo, user_hi, day, kind, lo_weight, hi_weight)
         SELECT $1, $2, $6, $7, $8, $9 FROM pair
         ON CONFLICT (user_lo, user_hi, day, kind) DO UPDATE SET
             lo_weight = friend_interaction_days.lo_weight + EXCLUDED.lo_weight,
             hi_weight = friend_interaction_days.hi_weight + EXCLUDED.hi_weight",
    )
    .bind(pair.lo)
    .bind(pair.hi)
    .bind(at)
    .bind(ranking::week_start(day))
    .bind(seconds_together)
    .bind(day)
    .bind(kind.bucket().code())
    .bind(lo_weight)
    .bind(hi_weight)
    .execute(&core.db)
    .await?;

    let are_friends = recorded.rows_affected() > 0;
    if are_friends && kind == InteractionKind::TimeTogether {
        record_night_minutes(core, pair, from, weight, at).await?;
    }
    Ok(())
}

/// Counts time together during the small hours towards a night spent
/// together. "Night" is judged in the reporting user's time zone.
async fn record_night_minutes(core: &Core, pair: Pair, reporter: Uuid, minutes: i32, at: DateTime<Utc>) -> AppResult<()> {
    let time_zone: Option<String> = sqlx::query_scalar("SELECT time_zone FROM users WHERE id = $1").bind(reporter).fetch_optional(&core.db).await?;
    let time_zone: chrono_tz::Tz = time_zone.and_then(|tz| tz.parse().ok()).unwrap_or(chrono_tz::Asia::Tokyo);
    let local = at.with_timezone(&time_zone);
    if !ranking::is_night(local.time()) {
        return Ok(());
    }
    let mut tx = core.db.begin().await?;
    let previous: (i32, Option<chrono::NaiveDate>) =
        sqlx::query_as("SELECT night_minutes, night_date FROM friend_pairs WHERE user_lo = $1 AND user_hi = $2 FOR UPDATE")
            .bind(pair.lo)
            .bind(pair.hi)
            .fetch_one(&mut *tx)
            .await?;
    let progress = ranking::night_progress(previous, local.date_naive(), minutes);
    sqlx::query(
        "UPDATE friend_pairs SET night_minutes = $3, night_date = $4, nights_together = nights_together + $5
         WHERE user_lo = $1 AND user_hi = $2",
    )
    .bind(pair.lo)
    .bind(pair.hi)
    .bind(progress.minutes)
    .bind(progress.date)
    .bind(i32::from(progress.completed))
    .execute(&mut *tx)
    .await?;
    if progress.completed {
        // The night also enters the 30-day window (for the score and PARTY_PEOPLE).
        sqlx::query(
            "INSERT INTO friend_interaction_days (user_lo, user_hi, day, kind, lo_weight) VALUES ($1, $2, $3, $4, 1)
             ON CONFLICT (user_lo, user_hi, day, kind) DO UPDATE SET lo_weight = friend_interaction_days.lo_weight + 1",
        )
        .bind(pair.lo)
        .bind(pair.hi)
        .bind(at.date_naive())
        .bind(Bucket::Night.code())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_are_ordered_whatever_the_argument_order() {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        let pair = Pair::of(a, b);
        assert_eq!(pair, Pair::of(b, a));
        assert!(pair.lo < pair.hi);
    }
}
