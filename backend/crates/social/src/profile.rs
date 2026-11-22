//! What the social domain adds to identity's profiles and user search:
//! relationship, mutual friends, friendship details and counters.
//!
//! `Profile.distance_meters` is deliberately left alone: the geo domain fills
//! it, with ghost mode applied.
use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokuly_core::prelude::*;
use tokuly_identity::ProfileExtension;
use tokuly_proto::identity::{Profile, Relationship};

use crate::{Social, bumps, graph, invites};

/// Mutual friends shown on a profile (the count covers all of them).
const MUTUAL_PREVIEW: i64 = 6;

#[async_trait]
impl ProfileExtension for Social {
    async fn enrich_profile(&self, viewer: Option<Uuid>, owner: Uuid, profile: &mut Profile) -> AppResult<()> {
        let core = &self.core;
        let stats = profile.stats.get_or_insert_default();
        stats.friend_count = graph::friend_count(core, owner).await? as i32;
        stats.intro_count = invites::intro_count(core, owner).await? as i32;
        stats.bump_count = bumps::bump_count(core, owner).await? as i32;

        let Some(viewer) = viewer.filter(|viewer| *viewer != owner) else {
            // Anonymous viewers have no relationship; identity marks SELF itself.
            return Ok(());
        };
        let relationship = graph::relationship(core, viewer, owner).await?;
        profile.relationship = relationship.into();
        profile.mutual_friend_count = graph::mutual_friend_count(core, viewer, owner).await?;
        let preview = graph::mutual_friend_ids(core, viewer, owner, MUTUAL_PREVIEW).await?;
        profile.mutual_friends = tokuly_identity::summaries_ordered(core, &preview).await?;

        if relationship == Relationship::RELATIONSHIP_FRIENDS {
            let friendship: Option<(DateTime<Utc>, i32, bool)> =
                sqlx::query_as("SELECT created_at, best_friend_rank, favorite FROM friendships WHERE user_id = $1 AND friend_id = $2")
                    .bind(viewer)
                    .bind(owner)
                    .fetch_optional(&core.db)
                    .await?;
            if let Some((since, rank, favorite)) = friendship {
                profile.friends_since = ts(since);
                profile.best_friend = rank > 0 || favorite;
            }
        }
        Ok(())
    }

    async fn relationships(&self, viewer: Uuid, user_ids: &[Uuid]) -> AppResult<HashMap<Uuid, (Relationship, i32)>> {
        let core = &self.core;
        let relationships = graph::relationships(core, viewer, user_ids).await?;
        let mutual = graph::mutual_friend_counts(core, viewer, user_ids).await?;
        Ok(relationships.into_iter().map(|(id, relationship)| (id, (relationship, mutual.get(&id).copied().unwrap_or(0)))).collect())
    }

    /// A block in either direction hides the two from each other.
    async fn can_view(&self, viewer: Uuid, owner: Uuid) -> AppResult<bool> {
        Ok(!graph::is_blocked_either(&self.core, viewer, owner).await?)
    }
}
