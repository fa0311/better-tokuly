//! A player's quest profile: the row, and what other people see of it.
//!
//! The quest domain shows itself on identity's profiles through two cards:
//!
//! - `quest.level` — level, the selected title and the showcased badges,
//!   for everyone who may see the profile (unless the owner switched
//!   `show_on_profile` off);
//! - `quest.streak` — the streak between the viewer and the owner, only
//!   when the two are friends and have one. It belongs to both, so the
//!   owner's `show_on_profile` does not hide it.
use async_trait::async_trait;
use buffa::Enumeration as _;
use chrono::NaiveDate;
use sqlx::PgConnection;
use tokuly_core::prelude::*;
use tokuly_identity::ProfileExtension;
use tokuly_proto::{
    identity::{Profile, ProfileCard},
    quest::{self as pb, StreakState},
};
use tokuly_social::graph;

use crate::{
    Quest,
    badges::{self, catalog},
    text::localized_text,
};

/// Most badges in a showcase.
pub(crate) const MAX_SHOWCASE: usize = 5;

/// Position of the quest cards among a profile's cards.
const LEVEL_CARD_ORDER: i32 = 100;
const STREAK_CARD_ORDER: i32 = 110;

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ProfileRow {
    pub user_id: Uuid,
    pub xp: i64,
    pub level: i32,
    pub rewarded_level: i32,
    pub title_badge: String,
    pub showcase: Vec<String>,
    pub last_active_day: Option<NaiveDate>,
    pub show_on_leaderboards: bool,
    pub show_on_profile: bool,
    pub streak_reminders: bool,
}

const PROFILE_COLUMNS: &str = "user_id, xp, level, rewarded_level, title_badge, showcase, last_active_day, show_on_leaderboards, show_on_profile, streak_reminders";

impl ProfileRow {
    /// The profile of a player who never earned or set anything.
    pub(crate) fn blank(user_id: Uuid) -> Self {
        Self {
            user_id,
            xp: 0,
            level: 1,
            rewarded_level: 1,
            title_badge: String::new(),
            showcase: Vec::new(),
            last_active_day: None,
            show_on_leaderboards: true,
            show_on_profile: true,
            streak_reminders: true,
        }
    }

    pub(crate) fn preferences(&self) -> pb::Preferences {
        pb::Preferences {
            show_on_leaderboards: self.show_on_leaderboards,
            show_on_profile: self.show_on_profile,
            streak_reminders: self.streak_reminders,
            ..Default::default()
        }
    }

    /// The selected title, if the badge still exists and has one.
    pub(crate) fn title(&self) -> Option<pb::Title> {
        title_of(&self.title_badge)
    }
}

/// The title a badge grants, as the API reports it.
pub(crate) fn title_of(badge_key: &str) -> Option<pb::Title> {
    let badge = catalog::find(badge_key)?;
    Some(pb::Title { badge_key: badge.key.to_owned(), text: badge.title.proto_opt()?.into(), ..Default::default() })
}

/// Creates the player's profile row if needed and locks it for the rest of
/// the transaction. Everything that changes a player's numbers starts here,
/// so those changes happen one after the other.
pub(crate) async fn lock_profile(tx: &mut PgConnection, user_id: Uuid) -> AppResult<ProfileRow> {
    sqlx::query("INSERT INTO quest_profiles (user_id) VALUES ($1) ON CONFLICT (user_id) DO NOTHING")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {PROFILE_COLUMNS} FROM quest_profiles WHERE user_id = $1 FOR UPDATE"
    )))
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?)
}

impl Quest {
    /// A player's profile; the blank one when they have none yet.
    pub(crate) async fn profile(&self, user_id: Uuid) -> AppResult<ProfileRow> {
        let row: Option<ProfileRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {PROFILE_COLUMNS} FROM quest_profiles WHERE user_id = $1"
        )))
        .bind(user_id)
        .fetch_optional(&self.core.db)
        .await?;
        Ok(row.unwrap_or_else(|| ProfileRow::blank(user_id)))
    }

    /// The profiles of several players, keyed by user (blank ones are absent).
    pub(crate) async fn profiles(&self, user_ids: &[Uuid]) -> AppResult<std::collections::HashMap<Uuid, ProfileRow>> {
        let rows: Vec<ProfileRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {PROFILE_COLUMNS} FROM quest_profiles WHERE user_id = ANY($1)"
        )))
        .bind(user_ids)
        .fetch_all(&self.core.db)
        .await?;
        Ok(rows.into_iter().map(|row| (row.user_id, row)).collect())
    }

    /// Changes the given settings and returns the profile.
    pub(crate) async fn save_preferences(
        &self,
        user_id: Uuid,
        show_on_leaderboards: Option<bool>,
        show_on_profile: Option<bool>,
        streak_reminders: Option<bool>,
    ) -> AppResult<ProfileRow> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO quest_profiles (user_id, show_on_leaderboards, show_on_profile, streak_reminders)
             VALUES ($1, COALESCE($2, true), COALESCE($3, true), COALESCE($4, true))
             ON CONFLICT (user_id) DO UPDATE SET
                 show_on_leaderboards = COALESCE($2, quest_profiles.show_on_leaderboards),
                 show_on_profile = COALESCE($3, quest_profiles.show_on_profile),
                 streak_reminders = COALESCE($4, quest_profiles.streak_reminders)
             RETURNING {PROFILE_COLUMNS}"
        )))
        .bind(user_id)
        .bind(show_on_leaderboards)
        .bind(show_on_profile)
        .bind(streak_reminders)
        .fetch_one(&self.core.db)
        .await?)
    }

    /// What `viewer` sees of `owner`'s quest progress. The caller has
    /// checked that the viewer may see the owner at all.
    pub(crate) async fn quest_profile(&self, viewer: Option<Uuid>, owner: Uuid) -> AppResult<pb::QuestProfile> {
        let profile = self.profile(owner).await?;
        let user = tokuly_identity::summary(&self.core, owner).await?;
        let own = viewer == Some(owner);
        let streak = match viewer {
            Some(viewer) if !own && graph::are_friends(&self.core, viewer, owner).await? => {
                Some(self.streak_between(viewer, owner).await?)
            }
            _ => None,
        };
        if !profile.show_on_profile && !own {
            return Ok(pb::QuestProfile {
                user: user.into(),
                streak: streak.into(),
                hidden: true,
                ..Default::default()
            });
        }
        let showcase = self.badges_by_key(owner, &profile.showcase).await?;
        Ok(pb::QuestProfile {
            user: user.into(),
            level: profile.level,
            title: profile.title().into(),
            showcase,
            badge_count: self.badge_count(owner).await?,
            streak: streak.into(),
            ..Default::default()
        })
    }
}

/// The `quest.level` card.
fn level_card(handle: &str, profile: &ProfileRow, showcase: &[pb::Badge]) -> ProfileCard {
    let title = profile.title();
    let (title_ja, title_en) = title
        .as_ref()
        .and_then(|title| title.text.as_option())
        .map(|text| (text.ja.clone(), text.en.clone()))
        .unwrap_or_default();
    // What clients need to draw the badges without another request.
    let badges: Vec<serde_json::Value> = showcase
        .iter()
        .map(|badge| {
            let name = badge.name.as_option();
            serde_json::json!({
                "key": badge.key,
                "emoji": badge.emoji,
                "tier": badge.tier.to_i32(),
                "name_ja": name.map(|name| name.ja.as_str()).unwrap_or_default(),
                "name_en": name.map(|name| name.en.as_str()).unwrap_or_default(),
            })
        })
        .collect();
    let mut data = std::collections::HashMap::new();
    data.insert("level".to_owned(), profile.level.to_string());
    data.insert("xp".to_owned(), profile.xp.to_string());
    data.insert("title_ja".to_owned(), title_ja.clone());
    data.insert("title_en".to_owned(), title_en.clone());
    data.insert("title_badge".to_owned(), title.map(|title| title.badge_key).unwrap_or_default());
    data.insert("badges".to_owned(), serde_json::Value::Array(badges).to_string());
    ProfileCard {
        kind: "quest.level".to_owned(),
        title: localized_text(format!("レベル{}", profile.level), format!("Level {}", profile.level)).into(),
        text: localized_text(title_ja, title_en).into(),
        emoji: showcase
            .first()
            .map(|badge| badge.emoji.clone())
            .filter(|emoji| !emoji.is_empty())
            .unwrap_or_else(|| "⭐".to_owned()),
        path: format!("/u/{handle}/quest"),
        data: data.into_iter().collect(),
        order: LEVEL_CARD_ORDER,
        ..Default::default()
    }
}

/// The `quest.streak` card for a streak the viewer has with the owner.
fn streak_card(streak: &pb::Streak) -> Option<ProfileCard> {
    let state = streak.state.as_known()?;
    if !matches!(state, StreakState::STREAK_STATE_ACTIVE | StreakState::STREAK_STATE_EXPIRING) || streak.days < 1 {
        return None;
    }
    let expiring = state == StreakState::STREAK_STATE_EXPIRING;
    let text = if expiring {
        localized_text("もうすぐ途切れちゃう!", "About to expire!")
    } else {
        localized_text("ふたりのストリーク", "Your streak together")
    };
    let mut data = std::collections::HashMap::new();
    data.insert("days".to_owned(), streak.days.to_string());
    data.insert("best_days".to_owned(), streak.best_days.to_string());
    data.insert("state".to_owned(), state.to_i32().to_string());
    data.insert("waiting_on".to_owned(), streak.waiting_on.to_i32().to_string());
    data.insert("next_milestone".to_owned(), streak.next_milestone.to_string());
    if let Some(expires_at) = streak.expires_at.as_option() {
        data.insert("expires_at".to_owned(), from_ts(expires_at).to_rfc3339());
    }
    Some(ProfileCard {
        kind: "quest.streak".to_owned(),
        title: localized_text(format!("{}日連続", streak.days), format!("{}-day streak", streak.days)).into(),
        text: text.into(),
        emoji: if expiring { "⌛" } else { "🔥" }.to_owned(),
        path: "/quest/streaks".to_owned(),
        data: data.into_iter().collect(),
        order: STREAK_CARD_ORDER,
        ..Default::default()
    })
}

#[async_trait]
impl ProfileExtension for Quest {
    async fn enrich_profile(&self, viewer: Option<Uuid>, owner: Uuid, profile: &mut Profile) -> AppResult<()> {
        let own = viewer == Some(owner);
        let row = self.profile(owner).await?;
        if row.show_on_profile || own {
            let handle = profile.user.as_option().map(|user| user.handle.clone()).unwrap_or_default();
            let showcase = self.badges_by_key(owner, &row.showcase).await?;
            profile.cards.push(level_card(&handle, &row, &showcase));
        }
        if let Some(viewer) = viewer.filter(|_| !own)
            && graph::are_friends(&self.core, viewer, owner).await?
            && let Some(card) = streak_card(&self.streak_between(viewer, owner).await?)
        {
            profile.cards.push(card);
        }
        Ok(())
    }
}

impl Quest {
    /// The owned badges among `keys`, in that order, as the owner's badges.
    pub(crate) async fn badges_by_key(&self, owner: Uuid, keys: &[String]) -> AppResult<Vec<pb::Badge>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let owned = badges::owned(&self.core.db, owner).await?;
        let stats = self.badge_stats().await?;
        Ok(keys
            .iter()
            .filter_map(|key| {
                let badge = catalog::find(key)?;
                let held = owned.get(badge.key)?;
                Some(badges::to_proto(badge, Some(held), None, stats.get(badge.key)))
            })
            .collect())
    }

    pub(crate) async fn badge_count(&self, owner: Uuid) -> AppResult<i32> {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM quest_badges WHERE user_id = $1")
            .bind(owner)
            .fetch_one(&self.core.db)
            .await?;
        Ok(count as i32)
    }
}

#[cfg(test)]
mod tests {
    use tokuly_proto::quest::{BadgeTier, StreakTurn};

    use super::*;

    #[test]
    fn the_level_card_carries_level_title_and_showcase() {
        let mut row = ProfileRow::blank(Uuid::now_v7());
        row.level = 12;
        row.xp = 4000;
        row.title_badge = "chatter_gold".to_owned();
        let showcase = vec![pb::Badge {
            key: "streak_gold".into(),
            emoji: "🔥".into(),
            tier: BadgeTier::BADGE_TIER_GOLD.into(),
            name: localized_text("ストリーク", "Streak").into(),
            ..Default::default()
        }];
        let card = level_card("mika", &row, &showcase);
        assert_eq!(card.kind, "quest.level");
        assert_eq!(card.title.as_option().unwrap().ja, "レベル12");
        assert_eq!(card.text.as_option().unwrap().ja, "おしゃべり番長");
        assert_eq!(card.emoji, "🔥");
        assert_eq!(card.path, "/u/mika/quest");
        assert_eq!(card.data["level"], "12");
        assert_eq!(card.data["title_en"], "Chatterbox-in-Chief");
        assert_eq!(card.data["title_badge"], "chatter_gold");
        let badges: serde_json::Value = serde_json::from_str(&card.data["badges"]).unwrap();
        assert_eq!(badges[0]["key"], "streak_gold");
        assert_eq!(badges[0]["tier"], 3);
        assert_eq!(badges[0]["name_ja"], "ストリーク");
    }

    #[test]
    fn a_blank_profile_still_makes_a_card() {
        let card = level_card("new", &ProfileRow::blank(Uuid::now_v7()), &[]);
        assert_eq!(card.title.as_option().unwrap().en, "Level 1");
        assert_eq!(card.emoji, "⭐");
        assert_eq!(card.data["title_ja"], "");
        assert_eq!(card.data["badges"], "[]");
    }

    #[test]
    fn only_running_streaks_make_a_card() {
        let running = pb::Streak {
            days: 12,
            best_days: 20,
            state: StreakState::STREAK_STATE_ACTIVE.into(),
            waiting_on: StreakTurn::STREAK_TURN_MINE.into(),
            next_milestone: 30,
            ..Default::default()
        };
        let card = streak_card(&running).unwrap();
        assert_eq!((card.kind.as_str(), card.emoji.as_str()), ("quest.streak", "🔥"));
        assert_eq!(card.title.as_option().unwrap().ja, "12日連続");
        assert_eq!(card.data["days"], "12");
        let expiring = pb::Streak { state: StreakState::STREAK_STATE_EXPIRING.into(), ..running.clone() };
        assert_eq!(streak_card(&expiring).unwrap().emoji, "⌛");
        assert!(
            streak_card(&pb::Streak {
                state: StreakState::STREAK_STATE_LOST.into(),
                lost_days: 12,
                ..Default::default()
            })
            .is_none()
        );
        assert!(
            streak_card(&pb::Streak { state: StreakState::STREAK_STATE_NONE.into(), ..Default::default() }).is_none()
        );
    }

    #[test]
    fn titles_come_from_badges_that_have_one() {
        assert_eq!(title_of("chatter_gold").unwrap().text.as_option().unwrap().en, "Chatterbox-in-Chief");
        assert!(title_of("chatter_bronze").is_none());
        assert!(title_of("no_such_badge").is_none());
        assert!(title_of("").is_none());
    }
}
