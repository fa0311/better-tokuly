//! "Accounts to follow": candidates from several signals, scored together.
//!
//! A signal is one [`FollowSignal`]: a reason, a weight, and a function that
//! returns candidates with a strength (how many of the user's friends follow
//! someone, ...). A candidate's score is the sum over the signals that found
//! them of `weight × ln(1 + strength)` ([`score`]): several signals beat one,
//! and a strength of 50 is not fifty times a strength of 1. A new signal is
//! one more implementation; other domains plug theirs in through
//! [`crate::Social::add_follow_signal`] (the community domain's
//! "popular in your communities" goes there, wired by the server).
//!
//! Built in: people who follow the user, friends of friends, accounts
//! followed by people the user follows, and accounts popular among the
//! user's schoolmates. Aggregates over other people's follows count a
//! candidate only when at least [`MIN_CROWD`] of them follow (one person's
//! follow is that person's information) and never count follows of locked
//! accounts the user cannot see the lists of.
//!
//! Suggested are only accounts the user has no tie with yet (no follow or
//! request, not a friend, no block either way, not dismissed), that are
//! active and discoverable, and in the user's own age group.
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use tokuly_core::prelude::*;
use tokuly_identity::{settings, users};
use tokuly_proto::social::FollowSuggestion;

use crate::{Social, audience::locked_sql, people};

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 50;
/// Candidates one signal may return.
pub const CANDIDATES_PER_SIGNAL: i64 = 200;
/// Fewest people behind an aggregate over other people's follows.
pub const MIN_CROWD: i64 = 3;
/// People shown in "followed by ...".
const FOLLOWED_BY_PREVIEW: usize = 3;

/// One source of follow suggestions.
#[async_trait]
pub trait FollowSignal: Send + Sync {
    /// The reason clients get in `FollowSuggestion.reasons` (SCREAMING_SNAKE).
    fn reason(&self) -> &'static str;
    /// How much this signal counts against the others.
    fn weight(&self) -> f64;
    /// Candidates for `user_id` with their strength (≥ 1). Need not filter:
    /// the user, people already followed, blocked and so on are dropped
    /// afterwards.
    async fn candidates(&self, core: &Core, user_id: Uuid) -> AppResult<Vec<(Uuid, u32)>>;
}

/// The score of a candidate found by signals of these (weight, strength).
pub fn score(found_by: &[(f64, u32)]) -> f64 {
    found_by.iter().map(|(weight, strength)| weight * f64::from(*strength).ln_1p()).sum()
}

/// People who follow the user and are not followed back.
struct FollowsYou;

#[async_trait]
impl FollowSignal for FollowsYou {
    fn reason(&self) -> &'static str {
        "FOLLOWS_YOU"
    }

    fn weight(&self) -> f64 {
        3.0
    }

    async fn candidates(&self, core: &Core, user_id: Uuid) -> AppResult<Vec<(Uuid, u32)>> {
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT follower_id FROM follows WHERE followee_id = $1 ORDER BY created_at DESC LIMIT $2",
        )
        .bind(user_id)
        .bind(CANDIDATES_PER_SIGNAL)
        .fetch_all(&core.db)
        .await?;
        Ok(ids.into_iter().map(|id| (id, 1)).collect())
    }
}

/// Friends of the user's friends; strength = friends in common.
struct FriendOfFriend;

#[async_trait]
impl FollowSignal for FriendOfFriend {
    fn reason(&self) -> &'static str {
        "FRIEND_OF_FRIEND"
    }

    fn weight(&self) -> f64 {
        2.0
    }

    async fn candidates(&self, core: &Core, user_id: Uuid) -> AppResult<Vec<(Uuid, u32)>> {
        counted(
            core,
            "SELECT theirs.friend_id, count(*) FROM friendships mine JOIN friendships theirs ON theirs.user_id = mine.friend_id
             WHERE mine.user_id = $1 AND theirs.friend_id <> $1
             GROUP BY theirs.friend_id ORDER BY count(*) DESC, theirs.friend_id LIMIT $2",
            user_id,
        )
        .await
    }
}

/// Accounts followed by people the user follows (whose lists the user can
/// see: they follow them). Strength = how many.
struct FollowedByFollowing;

#[async_trait]
impl FollowSignal for FollowedByFollowing {
    fn reason(&self) -> &'static str {
        "FOLLOWED_BY_FOLLOWING"
    }

    fn weight(&self) -> f64 {
        1.5
    }

    async fn candidates(&self, core: &Core, user_id: Uuid) -> AppResult<Vec<(Uuid, u32)>> {
        counted(
            core,
            "SELECT f.followee_id, count(*) FROM follows mine JOIN follows f ON f.follower_id = mine.followee_id
             WHERE mine.follower_id = $1 AND f.followee_id <> $1
             GROUP BY f.followee_id ORDER BY count(*) DESC, f.followee_id LIMIT $2",
            user_id,
        )
        .await
    }
}

/// Accounts many of the user's schoolmates follow (at least [`MIN_CROWD`],
/// and only follows of unlocked schoolmates).
struct PopularAtSchool;

#[async_trait]
impl FollowSignal for PopularAtSchool {
    fn reason(&self) -> &'static str {
        "POPULAR_AT_SCHOOL"
    }

    fn weight(&self) -> f64 {
        1.0
    }

    async fn candidates(&self, core: &Core, user_id: Uuid) -> AppResult<Vec<(Uuid, u32)>> {
        let schoolmates = users::schoolmates(core, user_id, 500).await?;
        if schoolmates.len() < MIN_CROWD as usize {
            return Ok(Vec::new());
        }
        let rows: Vec<(Uuid, i64)> = sqlx::query_as(concat!(
            "SELECT f.followee_id, count(*) FROM follows f
             WHERE f.follower_id = ANY($1) AND f.followee_id <> $2 AND NOT ",
            locked_sql!("f.follower_id"),
            " GROUP BY f.followee_id HAVING count(*) >= $3 ORDER BY count(*) DESC, f.followee_id LIMIT $4"
        ))
        .bind(&schoolmates)
        .bind(user_id)
        .bind(MIN_CROWD)
        .bind(CANDIDATES_PER_SIGNAL)
        .fetch_all(&core.db)
        .await?;
        Ok(rows.into_iter().map(|(id, count)| (id, count as u32)).collect())
    }
}

async fn counted(core: &Core, sql: &'static str, user_id: Uuid) -> AppResult<Vec<(Uuid, u32)>> {
    let rows: Vec<(Uuid, i64)> =
        sqlx::query_as(sql).bind(user_id).bind(CANDIDATES_PER_SIGNAL).fetch_all(&core.db).await?;
    Ok(rows.into_iter().map(|(id, count)| (id, count as u32)).collect())
}

/// The signals every deployment has.
pub(crate) fn built_in() -> Vec<Arc<dyn FollowSignal>> {
    vec![Arc::new(FollowsYou), Arc::new(FriendOfFriend), Arc::new(FollowedByFollowing), Arc::new(PopularAtSchool)]
}

/// A scored candidate: total, and its reasons strongest first.
#[derive(Debug, Clone, PartialEq)]
struct Scored {
    id: Uuid,
    score: f64,
    reasons: Vec<&'static str>,
}

/// Combines what the signals found into a ranking (best first; ties by ID).
fn rank(found: Vec<(&'static str, f64, Vec<(Uuid, u32)>)>) -> Vec<Scored> {
    let mut by_candidate: HashMap<Uuid, Vec<(&'static str, f64)>> = HashMap::new();
    for (reason, weight, candidates) in found {
        for (id, strength) in candidates {
            by_candidate.entry(id).or_default().push((reason, score(&[(weight, strength.max(1))])));
        }
    }
    let mut ranked: Vec<Scored> = by_candidate
        .into_iter()
        .map(|(id, mut parts)| {
            parts.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
            Scored {
                id,
                score: parts.iter().map(|(_, part)| part).sum(),
                reasons: parts.into_iter().map(|(reason, _)| reason).collect(),
            }
        })
        .collect();
    ranked.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.id.cmp(&b.id)));
    ranked
}

impl Social {
    /// Up to `limit` accounts `user_id` may want to follow (0 = the default).
    pub(crate) async fn list_follow_suggestions(&self, user_id: Uuid, limit: i32) -> AppResult<Vec<FollowSuggestion>> {
        let core = &self.core;
        core.cache.rate_limit(&format!("follow-suggestions:{user_id}"), 60, Duration::from_secs(60)).await?;
        let limit = if limit <= 0 { DEFAULT_LIMIT } else { (limit as usize).min(MAX_LIMIT) };

        let signals = self.follow_signals();
        let mut found = Vec::with_capacity(signals.len());
        for signal in &signals {
            match signal.candidates(core, user_id).await {
                Ok(candidates) => found.push((signal.reason(), signal.weight(), candidates)),
                // One failing signal (another domain's, say) must not empty the list.
                Err(e) => {
                    tracing::warn!(reason = signal.reason(), error = %e, "follow signal failed; suggesting without it")
                }
            }
        }
        let ranked = rank(found);
        let ids: Vec<Uuid> = ranked.iter().map(|s| s.id).collect();
        let suggestable = self.follow_suggestable_among(user_id, &ids).await?;
        let mut ranked: Vec<Scored> = ranked.into_iter().filter(|s| suggestable.contains(&s.id)).collect();
        ranked.truncate(limit);

        let ids: Vec<Uuid> = ranked.iter().map(|s| s.id).collect();
        let followed_by = self.followed_by_among(user_id, &ids).await?;
        let relations = self.follow_relations(user_id, &ids).await?;
        let mut needed = ids.clone();
        for preview in followed_by.values() {
            needed.extend(preview.iter().take(FOLLOWED_BY_PREVIEW));
        }
        let summaries = tokuly_identity::summaries(core, &needed).await?;
        Ok(ranked
            .into_iter()
            .map(|s| {
                let shared = followed_by.get(&s.id).map(Vec::as_slice).unwrap_or_default();
                FollowSuggestion {
                    user: users::pick(&summaries, s.id).into(),
                    reasons: s.reasons.into_iter().map(str::to_owned).collect(),
                    followed_by: shared
                        .iter()
                        .take(FOLLOWED_BY_PREVIEW)
                        .map(|id| users::pick(&summaries, *id))
                        .collect(),
                    followed_by_count: shared.len() as i32,
                    relation: relations.get(&s.id).cloned().unwrap_or_default().into(),
                    ..Default::default()
                }
            })
            .collect())
    }

    /// For each of `others`: the people `viewer` follows who follow them.
    async fn followed_by_among(&self, viewer: Uuid, others: &[Uuid]) -> AppResult<HashMap<Uuid, Vec<Uuid>>> {
        if others.is_empty() {
            return Ok(HashMap::new());
        }
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT f.followee_id, f.follower_id FROM follows f
             JOIN follows mine ON mine.follower_id = $1 AND mine.followee_id = f.follower_id
             JOIN users u ON u.id = f.follower_id
             WHERE f.followee_id = ANY($2) AND u.deleted_at IS NULL AND u.banned_at IS NULL
             ORDER BY mine.created_at",
        )
        .bind(viewer)
        .bind(others)
        .fetch_all(&self.core.db)
        .await?;
        let mut by_other: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        for (other, follower) in rows {
            by_other.entry(other).or_default().push(follower);
        }
        Ok(by_other)
    }

    /// The members of `candidates` that may be suggested to `user_id`.
    async fn follow_suggestable_among(&self, user_id: Uuid, candidates: &[Uuid]) -> AppResult<HashSet<Uuid>> {
        if candidates.is_empty() {
            return Ok(HashSet::new());
        }
        let core = &self.core;
        let unrelated: Vec<Uuid> = sqlx::query_scalar(
            "SELECT c.id FROM unnest($2::uuid[]) AS c(id)
             WHERE c.id <> $1
               AND NOT EXISTS (SELECT 1 FROM follows f WHERE f.follower_id = $1 AND f.followee_id = c.id)
               AND NOT EXISTS (SELECT 1 FROM follow_requests r WHERE r.follower_id = $1 AND r.followee_id = c.id)
               AND NOT EXISTS (SELECT 1 FROM friendships f WHERE f.user_id = $1 AND f.friend_id = c.id)
               AND NOT EXISTS (SELECT 1 FROM user_blocks b
                               WHERE (b.blocker_id = $1 AND b.blocked_id = c.id) OR (b.blocker_id = c.id AND b.blocked_id = $1))
               AND NOT EXISTS (SELECT 1 FROM follow_suggestion_dismissals d WHERE d.user_id = $1 AND d.dismissed_id = c.id)",
        )
        .bind(user_id)
        .bind(candidates)
        .fetch_all(&core.db)
        .await?;
        let active = users::filter_active(core, &unrelated).await?;
        let privacy = settings::privacy_of_many(core, &active).await?;
        let mut everyone = active.clone();
        everyone.push(user_id);
        let minors = people::minors_among(core, &everyone).await?;
        let user_is_minor = minors.contains(&user_id);
        Ok(active
            .into_iter()
            .filter(|id| {
                privacy.get(id).is_some_and(settings::findable_by_anyone) && minors.contains(id) == user_is_minor
            })
            .collect())
    }

    /// Stops suggesting `dismissed` to `user_id` as someone to follow.
    pub(crate) async fn dismiss_follow_suggestion(&self, user_id: Uuid, dismissed: Uuid) -> AppResult<()> {
        if user_id == dismissed {
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO follow_suggestion_dismissals (user_id, dismissed_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(user_id)
        .bind(dismissed)
        .execute(&self.core.db)
        .await
        .map_err(|e| {
            let e = AppError::from(e);
            if e.is_not_found() { AppError::not_found("USER_NOT_FOUND", "user not found") } else { e }
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn several_signals_beat_one_and_strength_has_diminishing_returns() {
        assert_eq!(score(&[]), 0.0);
        assert!(score(&[(1.0, 2)]) < 2.0 * score(&[(1.0, 1)]), "twice the strength is less than twice the score");
        assert!(score(&[(1.0, 50)]) < 10.0 * score(&[(1.0, 1)]));
        assert!(
            score(&[(2.0, 1), (1.5, 1)]) > score(&[(2.0, 2)]),
            "a second reason beats a second person behind the same one"
        );
    }

    #[test]
    fn candidates_are_ranked_by_their_summed_score_with_reasons_strongest_first() {
        let (both, strong, weak) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let ranked = rank(vec![
            ("FRIEND_OF_FRIEND", 2.0, vec![(both, 1), (strong, 5)]),
            ("FOLLOWED_BY_FOLLOWING", 1.5, vec![(both, 4), (weak, 1)]),
        ]);
        let order: Vec<Uuid> = ranked.iter().map(|s| s.id).collect();
        assert_eq!(order, vec![both, strong, weak]);
        assert_eq!(ranked[0].reasons, vec!["FOLLOWED_BY_FOLLOWING", "FRIEND_OF_FRIEND"], "1.5·ln5 > 2·ln2");
    }
}
