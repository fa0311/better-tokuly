//! "People you may know": friends of friends, address-book matches, invite
//! relations, schoolmates and hints from other domains.
//!
//! Friends of friends and schoolmates are derived when the list is read.
//! Everything else is a stored hint (`friend_suggestions`), written by
//! contact matching, by invites, or by another domain through [`suggest`].
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use tokuly_core::prelude::*;
use tokuly_identity::{settings, users};
use tokuly_proto::social::{ContactMatch, Suggestion};

use crate::{Social, graph};

const DEFAULT_SUGGESTIONS: usize = 20;
const MAX_SUGGESTIONS: usize = 50;
/// Candidates fetched per source before filtering.
const CANDIDATES_PER_SOURCE: i64 = 100;
/// Mutual friends shown with one suggestion.
const MUTUAL_PREVIEW: usize = 3;
/// Most address-book hashes accepted per `MatchContacts` call.
const MAX_CONTACT_HASHES: usize = 2000;

/// Why a user is suggested. The order is the display priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SuggestionReason {
    /// The suggested user invited this user.
    InvitedBy,
    /// The suggested user joined through this user's invite.
    Invited,
    /// The suggested user is in this user's address book.
    Contact,
    /// The two are often at the same place (reported by the geo domain).
    NearbyOften,
    FriendOfFriend,
    SameSchool,
}

impl SuggestionReason {
    /// The string clients receive in `Suggestion.reason`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvitedBy => "INVITED_BY",
            Self::Invited => "INVITED",
            Self::Contact => "CONTACT",
            Self::NearbyOften => "NEARBY_OFTEN",
            Self::FriendOfFriend => "FRIEND_OF_FRIEND",
            Self::SameSchool => "SAME_SCHOOL",
        }
    }

    fn from_stored(reason: &str) -> Option<Self> {
        [Self::InvitedBy, Self::Invited, Self::Contact, Self::NearbyOften].into_iter().find(|r| r.as_str() == reason)
    }
}

/// Stores a hint that `suggested_id` may be someone `user_id` knows. For
/// other domains (e.g. geo reporting `NearbyOften`); harmless to repeat.
/// The usual filters still apply when suggestions are listed.
pub async fn suggest(core: &Core, user_id: Uuid, suggested_id: Uuid, reason: SuggestionReason) -> AppResult<()> {
    if user_id == suggested_id {
        return Ok(());
    }
    sqlx::query("INSERT INTO friend_suggestions (user_id, suggested_id, reason) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING")
        .bind(user_id)
        .bind(suggested_id)
        .bind(reason.as_str())
        .execute(&core.db)
        .await?;
    Ok(())
}

/// Checks and normalizes one address-book hash: 64 hex digits (SHA-256).
fn normalize_contact_hash(hash: &str) -> AppResult<String> {
    let hash = hash.trim();
    ensure!(
        hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
        invalid,
        "INVALID_CONTACT_HASH",
        "contact hashes are SHA-256 digests in hex"
    );
    Ok(hash.to_ascii_lowercase())
}

/// Orders candidates for display: by reason priority, then by how many
/// friends they share with the user, then by ID for a stable result.
fn rank_candidates(candidates: &mut [(Uuid, SuggestionReason)], mutual_counts: &HashMap<Uuid, usize>) {
    candidates.sort_by(|(a, reason_a), (b, reason_b)| {
        let (mutual_a, mutual_b) = (mutual_counts.get(a).copied().unwrap_or(0), mutual_counts.get(b).copied().unwrap_or(0));
        reason_a.cmp(reason_b).then(mutual_b.cmp(&mutual_a)).then(a.cmp(b))
    });
}

impl Social {
    /// Up to `limit` people `user_id` may know (0 = the default).
    pub(crate) async fn list_suggestions(&self, user_id: Uuid, limit: i32) -> AppResult<Vec<Suggestion>> {
        let core = &self.core;
        core.cache.rate_limit(&format!("suggestions:{user_id}"), 60, Duration::from_secs(60)).await?;
        let limit = if limit <= 0 { DEFAULT_SUGGESTIONS } else { (limit as usize).min(MAX_SUGGESTIONS) };

        // 1. Gather candidates; a user found by several sources keeps the
        //    highest-priority reason.
        let mut reasons: HashMap<Uuid, SuggestionReason> = HashMap::new();
        let mut consider = |candidate: Uuid, reason: SuggestionReason| {
            reasons.entry(candidate).and_modify(|current| *current = (*current).min(reason)).or_insert(reason);
        };
        let hints: Vec<(Uuid, String)> =
            sqlx::query_as("SELECT suggested_id, reason FROM friend_suggestions WHERE user_id = $1 ORDER BY created_at DESC LIMIT $2")
                .bind(user_id)
                .bind(CANDIDATES_PER_SOURCE)
                .fetch_all(&core.db)
                .await?;
        for (candidate, reason) in hints {
            if let Some(reason) = SuggestionReason::from_stored(&reason) {
                consider(candidate, reason);
            }
        }
        let friends_of_friends: Vec<Uuid> = sqlx::query_scalar(
            "SELECT theirs.friend_id
             FROM friendships mine JOIN friendships theirs ON theirs.user_id = mine.friend_id
             WHERE mine.user_id = $1 AND theirs.friend_id <> $1
             GROUP BY theirs.friend_id
             ORDER BY count(*) DESC, theirs.friend_id
             LIMIT $2",
        )
        .bind(user_id)
        .bind(CANDIDATES_PER_SOURCE)
        .fetch_all(&core.db)
        .await?;
        for candidate in friends_of_friends {
            consider(candidate, SuggestionReason::FriendOfFriend);
        }
        for candidate in users::schoolmates(core, user_id, CANDIDATES_PER_SOURCE).await? {
            consider(candidate, SuggestionReason::SameSchool);
        }
        reasons.remove(&user_id);

        // 2. Drop everyone who should not be suggested.
        let candidate_ids: Vec<Uuid> = reasons.keys().copied().collect();
        let suggestable = self.suggestable_among(user_id, &candidate_ids).await?;
        let mut candidates: Vec<(Uuid, SuggestionReason)> = reasons.into_iter().filter(|(id, _)| suggestable.contains(id)).collect();

        // 3. Rank, cut, and decorate with mutual friends.
        let ids: Vec<Uuid> = candidates.iter().map(|(id, _)| *id).collect();
        let mutual = self.mutual_friends_with(user_id, &ids).await?;
        let mutual_counts: HashMap<Uuid, usize> = mutual.iter().map(|(id, friends)| (*id, friends.len())).collect();
        rank_candidates(&mut candidates, &mutual_counts);
        candidates.truncate(limit);

        let mut needed: Vec<Uuid> = candidates.iter().map(|(id, _)| *id).collect();
        for (id, _) in &candidates {
            needed.extend(mutual.get(id).into_iter().flatten().take(MUTUAL_PREVIEW));
        }
        let summaries = tokuly_identity::summaries(core, &needed).await?;
        Ok(candidates
            .into_iter()
            .map(|(id, reason)| {
                let shared = mutual.get(&id).map(Vec::as_slice).unwrap_or_default();
                Suggestion {
                    user: users::pick(&summaries, id).into(),
                    reason: reason.as_str().to_owned(),
                    mutual_friend_count: shared.len() as i32,
                    mutual_friends: shared.iter().take(MUTUAL_PREVIEW).map(|friend| users::pick(&summaries, *friend)).collect(),
                    ..Default::default()
                }
            })
            .collect())
    }

    /// The members of `candidates` that may be suggested to `user_id`: active
    /// and discoverable accounts with which the user has no relationship yet
    /// (friend, request in either direction, block in either direction) and
    /// which the user has not dismissed.
    async fn suggestable_among(&self, user_id: Uuid, candidates: &[Uuid]) -> AppResult<HashSet<Uuid>> {
        if candidates.is_empty() {
            return Ok(HashSet::new());
        }
        let core = &self.core;
        let unrelated: Vec<Uuid> = sqlx::query_scalar(
            "SELECT c.id FROM unnest($2::uuid[]) AS c(id)
             WHERE NOT EXISTS (SELECT 1 FROM friendships f WHERE f.user_id = $1 AND f.friend_id = c.id)
               AND NOT EXISTS (SELECT 1 FROM friend_requests r
                               WHERE (r.from_user_id = $1 AND r.to_user_id = c.id) OR (r.from_user_id = c.id AND r.to_user_id = $1))
               AND NOT EXISTS (SELECT 1 FROM user_blocks b
                               WHERE (b.blocker_id = $1 AND b.blocked_id = c.id) OR (b.blocker_id = c.id AND b.blocked_id = $1))
               AND NOT EXISTS (SELECT 1 FROM friend_suggestion_dismissals d WHERE d.user_id = $1 AND d.dismissed_id = c.id)",
        )
        .bind(user_id)
        .bind(candidates)
        .fetch_all(&core.db)
        .await?;
        let active = users::filter_active(core, &unrelated).await?;
        let privacy = settings::privacy_of_many(core, &active).await?;
        Ok(active.into_iter().filter(|id| privacy.get(id).is_some_and(|p| p.discoverable)).collect())
    }

    /// The friends `user_id` shares with each of `others` (users without
    /// shared friends are absent).
    async fn mutual_friends_with(&self, user_id: Uuid, others: &[Uuid]) -> AppResult<HashMap<Uuid, Vec<Uuid>>> {
        if others.is_empty() {
            return Ok(HashMap::new());
        }
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT theirs.user_id, mine.friend_id
             FROM friendships mine JOIN friendships theirs ON theirs.friend_id = mine.friend_id
             WHERE mine.user_id = $1 AND theirs.user_id = ANY($2)
             ORDER BY mine.created_at, mine.friend_id",
        )
        .bind(user_id)
        .bind(others)
        .fetch_all(&self.core.db)
        .await?;
        let mut mutual: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        for (other, friend) in rows {
            mutual.entry(other).or_default().push(friend);
        }
        Ok(mutual)
    }

    /// Stops suggesting `dismissed` to `user_id`, for good.
    pub(crate) async fn dismiss_suggestion(&self, user_id: Uuid, dismissed: Uuid) -> AppResult<()> {
        if user_id == dismissed {
            return Ok(());
        }
        let mut tx = self.core.db.begin().await?;
        sqlx::query("INSERT INTO friend_suggestion_dismissals (user_id, dismissed_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(user_id)
            .bind(dismissed)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                let e = AppError::from(e);
                if e.is_not_found() { AppError::not_found("USER_NOT_FOUND", "user not found") } else { e }
            })?;
        sqlx::query("DELETE FROM friend_suggestions WHERE user_id = $1 AND suggested_id = $2").bind(user_id).bind(dismissed).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Finds the accounts behind address-book hashes (SHA-256 of lowercased
    /// emails). Only discoverable accounts with a verified email match. The
    /// hashes themselves are not kept; matches become `CONTACT` suggestions.
    pub(crate) async fn match_contacts(&self, user_id: Uuid, hashes: &[&str]) -> AppResult<Vec<ContactMatch>> {
        let core = &self.core;
        ensure!(hashes.len() <= MAX_CONTACT_HASHES, invalid, "TOO_MANY_CONTACTS", "at most {MAX_CONTACT_HASHES} contacts per request");
        let mut wanted = hashes.iter().map(|hash| normalize_contact_hash(hash)).collect::<AppResult<Vec<String>>>()?;
        wanted.sort_unstable();
        wanted.dedup();
        if wanted.is_empty() {
            return Ok(Vec::new());
        }
        // One call covers a whole address book, so a few per day is plenty;
        // more would turn this into a way to test lists of addresses.
        core.cache.rate_limit(&format!("match-contacts:{user_id}"), 6, Duration::from_secs(3600)).await?;
        core.cache.rate_limit(&format!("match-contacts-day:{user_id}"), 30, Duration::from_secs(24 * 3600)).await?;

        // Identity owns the lookup (verified, active accounts by email hash);
        // which of them the caller gets to see is decided here.
        let found: Vec<(Uuid, String)> = users::find_by_contact_hashes(core, &wanted).await?.into_iter().filter(|(id, _)| *id != user_id).collect();
        let found_ids: Vec<Uuid> = found.iter().map(|(id, _)| *id).collect();
        let privacy = settings::privacy_of_many(core, &found_ids).await?;
        let hidden: HashSet<Uuid> = graph::blocked_ids_either(core, user_id).await?.into_iter().collect();
        let matches: Vec<(Uuid, String)> =
            found.into_iter().filter(|(id, _)| !hidden.contains(id) && privacy.get(id).is_some_and(|p| p.discoverable)).collect();

        let matched_ids: Vec<Uuid> = matches.iter().map(|(id, _)| *id).collect();
        sqlx::query("INSERT INTO friend_suggestions (user_id, suggested_id, reason) SELECT $1, id, $3 FROM unnest($2::uuid[]) AS m(id) ON CONFLICT DO NOTHING")
            .bind(user_id)
            .bind(&matched_ids)
            .bind(SuggestionReason::Contact.as_str())
            .execute(&core.db)
            .await?;
        let summaries = tokuly_identity::summaries(core, &matched_ids).await?;
        let relationships = graph::relationships(core, user_id, &matched_ids).await?;
        Ok(matches
            .into_iter()
            .map(|(id, hash)| ContactMatch {
                contact_hash: hash,
                user: users::pick(&summaries, id).into(),
                relationship: relationships.get(&id).copied().unwrap_or(tokuly_proto::identity::Relationship::RELATIONSHIP_NONE).into(),
                ..Default::default()
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contact_hashes_must_be_sha256_hex() {
        let hash = "A".repeat(64);
        assert_eq!(normalize_contact_hash(&format!(" {hash} ")).unwrap(), "a".repeat(64));
        for bad in ["", "abc", &"g".repeat(64), &"a".repeat(63), &"a".repeat(65)] {
            assert_eq!(normalize_contact_hash(bad).unwrap_err().reason, "INVALID_CONTACT_HASH");
        }
    }

    #[test]
    fn suggestions_are_ordered_by_reason_then_mutual_friends() {
        let (invited, contact, close, distant) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let mut candidates = vec![
            (distant, SuggestionReason::FriendOfFriend),
            (contact, SuggestionReason::Contact),
            (close, SuggestionReason::FriendOfFriend),
            (invited, SuggestionReason::InvitedBy),
        ];
        let mutual = HashMap::from([(close, 5), (distant, 1)]);
        rank_candidates(&mut candidates, &mutual);
        assert_eq!(candidates.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![invited, contact, close, distant]);
    }

    #[test]
    fn only_hint_reasons_are_read_back_from_storage() {
        assert_eq!(SuggestionReason::from_stored("CONTACT"), Some(SuggestionReason::Contact));
        assert_eq!(SuggestionReason::from_stored("NEARBY_OFTEN"), Some(SuggestionReason::NearbyOften));
        assert_eq!(SuggestionReason::from_stored("FRIEND_OF_FRIEND"), None, "derived at read time, never stored");
        assert_eq!(SuggestionReason::from_stored("???"), None);
    }
}
