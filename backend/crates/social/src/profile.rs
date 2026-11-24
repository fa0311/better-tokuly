//! What the social domain adds to identity's profiles and user search:
//! relationship, mutual friends, friendship details and counters, and a
//! `social.follow` card with follower / following counts and the viewer's
//! follow state.
//!
//! `Profile.distance_meters` is deliberately left alone: the geo domain fills
//! it, with ghost mode applied.
use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokuly_core::prelude::*;
use tokuly_identity::ProfileExtension;
use tokuly_proto::{
    common::LocalizedText,
    identity::{Profile, ProfileCard, Relationship},
    social::FollowState,
};

use crate::{Social, bumps, graph, invites};

/// Mutual friends shown on a profile (the count covers all of them).
const MUTUAL_PREVIEW: i64 = 6;

#[async_trait]
impl ProfileExtension for Social {
    async fn enrich_profile(&self, viewer: Option<Uuid>, owner: Uuid, profile: &mut Profile) -> AppResult<()> {
        let core = &self.core;
        let stats = profile.stats.get_or_insert_default();
        stats.bump_count = bumps::bump_count(core, owner).await? as i32;
        // Friend and intro counts are for signed-in viewers only, and leave
        // out whom that viewer's lists leave out (people in a block with the
        // viewer). A signed-out count would include them, and comparing the
        // two would tell a blocked user someone blocked them. (Comparing
        // with what a second account sees cannot be prevented.)
        if let Some(viewer) = viewer {
            let hidden = if viewer == owner { Vec::new() } else { graph::blocked_ids_either(core, viewer).await? };
            stats.friend_count = graph::visible_friend_count(core, owner, &hidden).await? as i32;
            stats.intro_count = invites::visible_intro_count(core, owner, &hidden).await? as i32;
        }

        if let Some(viewer) = viewer {
            profile.cards.push(self.follow_card(viewer, owner, profile).await?);
        }

        let Some(viewer) = viewer.filter(|viewer| *viewer != owner) else {
            // Anonymous viewers have no relationship; identity marks SELF itself.
            return Ok(());
        };
        let relationship = graph::relationship(core, viewer, owner).await?;
        profile.relationship = relationship.into();
        // Mutual friends are part of the owner's friend list: shown only to
        // those who may see it.
        if !self.friend_lists_visible_to(viewer, &[owner]).await?.is_empty() {
            profile.mutual_friend_count = graph::mutual_friend_count(core, viewer, owner).await?;
            let preview = graph::mutual_friend_ids(core, viewer, owner, MUTUAL_PREVIEW).await?;
            profile.mutual_friends = tokuly_identity::summaries_ordered(core, &preview).await?;
        }

        if relationship == Relationship::RELATIONSHIP_FRIENDS {
            let friendship: Option<(DateTime<Utc>, i32, bool)> = sqlx::query_as(
                "SELECT created_at, best_friend_rank, favorite FROM friendships WHERE user_id = $1 AND friend_id = $2",
            )
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
        let mut mutual = graph::mutual_friend_counts(core, viewer, user_ids).await?;
        let visible = self.friend_lists_visible_to(viewer, user_ids).await?;
        mutual.retain(|id, _| visible.contains(id));
        Ok(relationships
            .into_iter()
            .map(|(id, relationship)| (id, (relationship, mutual.get(&id).copied().unwrap_or(0))))
            .collect())
    }

    /// A block in either direction hides the two from each other.
    /// (Follow counts and state live in the profile's `social.follow` card.)
    async fn can_view(&self, viewer: Uuid, owner: Uuid) -> AppResult<bool> {
        Ok(!graph::is_blocked_either(&self.core, viewer, owner).await?)
    }
}

/// Where the follow card sits among the cards (near the top).
const FOLLOW_CARD_ORDER: i32 = 5;

impl Social {
    /// The `social.follow` card: counts as the viewer sees them (signed-in
    /// viewers only, like the friend count) and the viewer's follow state.
    /// `data`: `follower_count`, `following_count`, `following` (NONE /
    /// FOLLOWING / REQUESTED), `follows_you`, `locked`, `known_follower_count`.
    async fn follow_card(&self, viewer: Uuid, owner: Uuid, profile: &Profile) -> AppResult<ProfileCard> {
        let summary = self.follow_summary(viewer, owner).await?;
        let relation = summary.relation.as_option().cloned().unwrap_or_default();
        let following = match relation.following.as_known() {
            Some(FollowState::FOLLOW_STATE_FOLLOWING) => "FOLLOWING",
            Some(FollowState::FOLLOW_STATE_REQUESTED) => "REQUESTED",
            _ => "NONE",
        };
        let handle = profile.user.as_option().map(|user| user.handle.clone()).unwrap_or_default();
        let (followers, following_count) = (summary.follower_count, summary.following_count);
        Ok(ProfileCard {
            kind: "social.follow".into(),
            title: LocalizedText { ja: "フォロー".into(), en: "Follows".into(), ..Default::default() }.into(),
            text: LocalizedText {
                ja: format!("フォロワー {followers}人 · フォロー中 {following_count}人"),
                en: format!("{followers} followers · {following_count} following"),
                ..Default::default()
            }
            .into(),
            emoji: "👥".into(),
            path: if handle.is_empty() { String::new() } else { format!("/u/{handle}/followers") },
            data: [
                ("follower_count".to_owned(), followers.to_string()),
                ("following_count".to_owned(), following_count.to_string()),
                ("following".to_owned(), following.to_owned()),
                ("follows_you".to_owned(), relation.follows_you.to_string()),
                ("locked".to_owned(), summary.locked.to_string()),
                ("known_follower_count".to_owned(), summary.known_follower_count.to_string()),
            ]
            .into_iter()
            .collect(),
            order: FOLLOW_CARD_ORDER,
            ..Default::default()
        })
    }
}
