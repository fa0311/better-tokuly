//! How a subject takes reactions ([`Rules`]), and the settings owners
//! choose for reactions to their content (`reaction_settings`).
//!
//! # One reaction or several
//!
//! Misskey lets a person put one reaction on a note; Discord lets them put
//! up to twenty different emoji on a message. Both are right for what they
//! are about:
//!
//! - content made for an audience (videos, shorts, notes, live streams,
//!   stamp packs, questions…) takes **one reaction per person**: a reaction
//!   there says how the person felt about the thing, every count is a
//!   number of people, and a creator reading "🎉 120" knows 120 people
//!   cheered. A new reaction replaces the previous one;
//! - the subjects of group conversation (community threads, replies and
//!   events, plans — `TOKULY_ENGAGEMENT_MULTI_REACTION_TYPES`) take
//!   **several distinct emoji per person**, up to
//!   `TOKULY_ENGAGEMENT_REACTIONS_PER_PERSON`: there reactions are quick
//!   replies ("✅ 👀 🙏" on an event), and several at once are normal.
//!   A like and a dislike still replace each other;
//! - comments take a like or a dislike and nothing else.
//!
//! Either way a subject carries at most
//! `TOKULY_ENGAGEMENT_REACTION_EMOJI_PER_SUBJECT` distinct emoji. Once it is
//! full only emoji already on it can be added, as on Discord, so a summary
//! stays readable and nobody can spam a subject with a hundred different
//! emoji.
//!
//! # Owner settings
//!
//! Owners decide whether their subjects take custom emoji at all (default
//! yes) and whether they take custom emoji flagged sensitive (default no:
//! many of the people here are minors, and the owner is the one who has to
//! look at what is put under their post). Content that belongs to nobody
//! takes custom emoji but never sensitive ones.
use std::{collections::HashMap, time::Duration};

use tokuly_core::prelude::*;
use tokuly_proto::engagement::{ReactionRules, ReactionSettings};

use crate::{
    Engagement,
    subjects::{Subject, SubjectKey},
};

/// Changes of one user's settings per minute.
const SETTING_CHANGES_PER_MINUTE: u32 = 30;

/// What an owner chose for reactions to their content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::FromRow)]
pub(crate) struct Settings {
    pub custom_emoji: bool,
    pub sensitive_emoji: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { custom_emoji: true, sensitive_emoji: false }
    }
}

impl Settings {
    pub(crate) fn to_proto(self) -> ReactionSettings {
        ReactionSettings {
            custom_emoji: self.custom_emoji,
            sensitive_emoji: self.sensitive_emoji,
            ..Default::default()
        }
    }
}

/// How one subject takes reactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rules {
    /// Emoji are taken at all.
    pub emoji: bool,
    /// One person may hold several distinct emoji.
    pub multiple: bool,
    pub max_per_person: i32,
    pub max_distinct_emoji: i32,
    pub custom_emoji: bool,
    pub sensitive_emoji: bool,
}

impl Rules {
    pub(crate) fn to_proto(self) -> ReactionRules {
        ReactionRules {
            emoji: self.emoji,
            multiple: self.multiple,
            max_per_person: self.max_per_person,
            max_distinct_emoji: self.max_distinct_emoji,
            custom_emoji: self.custom_emoji,
            sensitive_emoji: self.sensitive_emoji,
            ..Default::default()
        }
    }
}

/// The settings of `owners` (absent ones have the defaults).
pub(crate) async fn settings_of(core: &Core, owners: &[Uuid]) -> AppResult<HashMap<Uuid, Settings>> {
    let owners: Vec<Uuid> = owners.iter().copied().filter(|owner| !owner.is_nil()).collect();
    if owners.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(Uuid, bool, bool)> =
        sqlx::query_as("SELECT user_id, custom_emoji, sensitive_emoji FROM reaction_settings WHERE user_id = ANY($1)")
            .bind(&owners)
            .fetch_all(&core.db)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(user_id, custom_emoji, sensitive_emoji)| (user_id, Settings { custom_emoji, sensitive_emoji }))
        .collect())
}

impl Engagement {
    /// The rules of a subject of `key`'s type whose owner chose `settings`
    /// (`None`: content that belongs to nobody).
    pub(crate) fn rules_for(&self, key: SubjectKey, settings: Option<Settings>) -> Rules {
        let emoji = !key.is_comment();
        let multiple = emoji && self.multi_reaction_types.contains(&key.kind);
        let settings = settings.unwrap_or(Settings { custom_emoji: true, sensitive_emoji: false });
        Rules {
            emoji,
            multiple,
            max_per_person: if multiple { self.cfg.reactions_per_person } else { 1 },
            max_distinct_emoji: self.cfg.reaction_emoji_per_subject,
            custom_emoji: emoji && settings.custom_emoji,
            sensitive_emoji: emoji && settings.custom_emoji && settings.sensitive_emoji,
        }
    }

    /// The rules of resolved subjects, keyed by subject; one query for the
    /// owners' settings.
    pub(crate) async fn rules_of(&self, subjects: &[&Subject]) -> AppResult<HashMap<SubjectKey, Rules>> {
        let owners: Vec<Uuid> = subjects.iter().filter_map(|subject| subject.owner()).collect();
        let settings = settings_of(&self.core, &owners).await?;
        Ok(subjects
            .iter()
            .map(|subject| {
                let chosen = subject.owner().map(|owner| settings.get(&owner).copied().unwrap_or_default());
                (subject.key, self.rules_for(subject.key, chosen))
            })
            .collect())
    }

    /// The rules of one resolved subject.
    pub(crate) async fn rules_of_subject(&self, subject: &Subject) -> AppResult<Rules> {
        Ok(self.rules_of(&[subject]).await?.remove(&subject.key).unwrap_or_else(|| self.rules_for(subject.key, None)))
    }

    /// The caller's settings (`ReactionService.GetReactionSettings`).
    pub(crate) async fn reaction_settings(&self, user_id: Uuid) -> AppResult<Settings> {
        Ok(settings_of(&self.core, &[user_id]).await?.remove(&user_id).unwrap_or_default())
    }

    /// Replaces the caller's settings (`ReactionService.UpdateReactionSettings`).
    pub(crate) async fn update_reaction_settings(&self, user_id: Uuid, settings: Settings) -> AppResult<Settings> {
        self.core
            .cache
            .rate_limit(
                &format!("engagement:reaction-settings:{user_id}"),
                SETTING_CHANGES_PER_MINUTE,
                Duration::from_secs(60),
            )
            .await?;
        let stored: Settings = sqlx::query_as(
            "INSERT INTO reaction_settings (user_id, custom_emoji, sensitive_emoji) VALUES ($1, $2, $3)
             ON CONFLICT (user_id) DO UPDATE SET custom_emoji = $2, sensitive_emoji = $3
             RETURNING custom_emoji, sensitive_emoji",
        )
        .bind(user_id)
        .bind(settings.custom_emoji)
        .bind(settings.sensitive_emoji)
        .fetch_one(&self.core.db)
        .await?;
        Ok(stored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default_to_custom_but_not_sensitive() {
        let settings = Settings::default();
        assert!(settings.custom_emoji && !settings.sensitive_emoji);
        let proto = settings.to_proto();
        assert!(proto.custom_emoji && !proto.sensitive_emoji);
    }
}
