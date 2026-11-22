//! Badges: awarding them, what a player owns, rarity.
//!
//! [`award`] is the public way to give a player a badge of the catalogue —
//! other domains call it for badges they decide about. Inside the quest
//! domain badges are awarded in the transaction of whatever earned them
//! ([`award_tx`]), so a badge and the progress behind it never disagree, and
//! announced once that transaction has committed ([`announce_unlocked`]).
use std::{collections::HashMap, sync::Arc};

use chrono::{Datelike as _, NaiveDate};
use sqlx::{PgConnection, PgPool};
use tokuly_core::{
    analytics::AnalyticsEvent,
    outbound::{self, ActivityAudience, ActivityEntry, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::quest::{self as pb, BadgeRarity, BadgeTier, quest_event::Event as Push};

use crate::{Quest, metrics::Metric, profile::lock_profile, store::DayCounters};

pub mod catalog;
mod svc;

use catalog::{BadgeDef, CATALOG, Criterion};

/// A badge as a player owns it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct OwnedBadge {
    pub badge_key: String,
    pub earned_at: Time,
    pub times: i32,
}

/// How many players own a badge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BadgeStat {
    /// Share of the active players owning it, 0-1.
    pub share: f64,
    /// False while too few players are active for a share to be published.
    pub known: bool,
}

/// Fewest active players for which a share is published. Below it, a share
/// would say who owns a badge rather than how rare it is (with three
/// players, "33 %" is one of them).
pub(crate) const MIN_RARITY_PLAYERS: i64 = 10;

/// The rarity class of an owner share.
pub(crate) fn rarity(stat: Option<&BadgeStat>) -> BadgeRarity {
    match stat {
        Some(stat) if stat.known => match stat.share {
            share if share >= 0.40 => BadgeRarity::BADGE_RARITY_COMMON,
            share if share >= 0.15 => BadgeRarity::BADGE_RARITY_UNCOMMON,
            share if share >= 0.05 => BadgeRarity::BADGE_RARITY_RARE,
            share if share >= 0.01 => BadgeRarity::BADGE_RARITY_EPIC,
            _ => BadgeRarity::BADGE_RARITY_LEGENDARY,
        },
        _ => BadgeRarity::BADGE_RARITY_UNSPECIFIED,
    }
}

fn tier_name(tier: BadgeTier) -> Option<(&'static str, &'static str)> {
    match tier {
        BadgeTier::BADGE_TIER_BRONZE => Some(("ブロンズ", "Bronze")),
        BadgeTier::BADGE_TIER_SILVER => Some(("シルバー", "Silver")),
        BadgeTier::BADGE_TIER_GOLD => Some(("ゴールド", "Gold")),
        _ => None,
    }
}

/// A badge as the API shows it for one player: `held` is that player's
/// copy (if any), `progress` how far they are on a counted badge.
pub(crate) fn to_proto(
    badge: &BadgeDef,
    held: Option<&OwnedBadge>,
    progress: Option<i64>,
    stat: Option<&BadgeStat>,
) -> pb::Badge {
    let target = match badge.criterion {
        Criterion::Count(_, threshold) => threshold,
        Criterion::Level(level) => level as i64,
        Criterion::StreakDays(days) => days as i64,
        Criterion::Streaks { count, .. } => count as i64,
        _ => 0,
    };
    let progress = if held.is_some() { target } else { progress.unwrap_or(0).clamp(0, target) };
    pb::Badge {
        key: badge.key.to_owned(),
        family: badge.family.def().key.to_owned(),
        line: badge.line.to_owned(),
        tier: badge.tier.into(),
        name: badge.name.proto().into(),
        description: badge.how.proto_with(badge.n).into(),
        emoji: badge.emoji.to_owned(),
        secret: badge.secret,
        title: badge.title.proto_opt().into(),
        rarity: rarity(stat).into(),
        owner_share: stat.filter(|stat| stat.known).map(|stat| stat.share).unwrap_or(0.0),
        owned: held.is_some(),
        earned_at: opt_ts(held.map(|held| held.earned_at)),
        progress,
        target,
        times: held.map(|held| held.times).unwrap_or(0),
        ..Default::default()
    }
}

/// A secret badge as shown to somebody who does not own it: that it exists
/// and how rare it is, nothing about what it is.
pub(crate) fn masked(badge: &BadgeDef, stat: Option<&BadgeStat>) -> pb::Badge {
    pb::Badge {
        // Not the real key either: `birthday` would give the game away.
        key: catalog::masked_key(badge),
        family: badge.family.def().key.to_owned(),
        tier: badge.tier.into(),
        secret: true,
        rarity: rarity(stat).into(),
        owner_share: stat.filter(|stat| stat.known).map(|stat| stat.share).unwrap_or(0.0),
        ..Default::default()
    }
}

/// The badges a player owns, by key.
pub(crate) async fn owned(db: &PgPool, user_id: Uuid) -> AppResult<HashMap<String, OwnedBadge>> {
    let rows: Vec<OwnedBadge> =
        sqlx::query_as("SELECT badge_key, earned_at, times FROM quest_badges WHERE user_id = $1")
            .bind(user_id)
            .fetch_all(db)
            .await?;
    Ok(rows.into_iter().map(|row| (row.badge_key.clone(), row)).collect())
}

/// Gives `user_id` the badge `badge_key` of the catalogue. Returns whether
/// the player got it now (`false`: they had it already). The player is told
/// (notification `quest.badge`, realtime `QuestEvent.badge_unlocked`); a
/// badge that comes with signing up (`BadgeDef::quiet`) is pushed to open
/// clients only.
///
/// This is the quest domain's API for other domains: a domain that decides
/// about a badge itself (an event prize, a moderation thank-you) adds the
/// badge to the catalogue and calls this.
///
/// Errors: `BADGE_NOT_FOUND` (no such key), `USER_NOT_FOUND` (no such
/// account, or it was deleted).
pub async fn award(core: &Core, user_id: Uuid, badge_key: &str) -> AppResult<bool> {
    let badge = catalog::find(badge_key).or_not_found("BADGE_NOT_FOUND", "no such badge")?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1 AND deleted_at IS NULL)")
        .bind(user_id)
        .fetch_one(&core.db)
        .await?;
    ensure!(exists, not_found, "USER_NOT_FOUND", "user not found");
    let mut tx = core.db.begin().await?;
    let awarded = award_tx(&mut tx, user_id, badge, serde_json::Value::Null).await?;
    tx.commit().await?;
    if awarded {
        announce_unlocked(core, user_id, badge).await;
    }
    Ok(awarded)
}

/// Awards a badge inside the caller's transaction, with the notification.
/// Returns whether the player got it now; the caller announces it after the
/// commit ([`announce_unlocked`]). Deleted accounts get nothing.
pub(crate) async fn award_tx(
    tx: &mut PgConnection,
    user_id: Uuid,
    badge: &BadgeDef,
    data: serde_json::Value,
) -> AppResult<bool> {
    let data = if data.is_null() { serde_json::json!({}) } else { data };
    let inserted = sqlx::query(
        "INSERT INTO quest_badges (user_id, badge_key, data)
         SELECT $1, $2, $3 WHERE EXISTS (SELECT 1 FROM users WHERE id = $1 AND deleted_at IS NULL)
         ON CONFLICT (user_id, badge_key) DO NOTHING",
    )
    .bind(user_id)
    .bind(badge.key)
    .bind(data)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;
    if inserted {
        notify_unlocked(tx, user_id, badge).await?;
    }
    Ok(inserted)
}

/// Awards a badge that can be won repeatedly (a season badge): the first
/// time it is a new badge, afterwards `times` grows. Returns whether it was
/// new. The player is notified either way.
pub(crate) async fn award_again_tx(
    tx: &mut PgConnection,
    user_id: Uuid,
    badge: &BadgeDef,
    data: serde_json::Value,
) -> AppResult<bool> {
    let new: Option<bool> = sqlx::query_scalar(
        "INSERT INTO quest_badges (user_id, badge_key, data)
         SELECT $1, $2, $3 WHERE EXISTS (SELECT 1 FROM users WHERE id = $1 AND deleted_at IS NULL)
         ON CONFLICT (user_id, badge_key) DO UPDATE SET times = quest_badges.times + 1, data = EXCLUDED.data
         RETURNING times = 1",
    )
    .bind(user_id)
    .bind(badge.key)
    .bind(data)
    .fetch_optional(&mut *tx)
    .await?;
    if new.is_some() {
        notify_unlocked(tx, user_id, badge).await?;
    }
    Ok(new.unwrap_or(false))
}

/// Who may see a player's quest achievements in feeds: their friends,
/// unless the player keeps their progress off their profile.
pub(crate) fn activity_audience(show_on_profile: bool) -> ActivityAudience {
    if show_on_profile { ActivityAudience::Friends } else { ActivityAudience::Private }
}

/// The `quest.badge` notification, and — for a badge that comes with a
/// title — an entry in the player's activity log for their friends' feed.
/// Secret badges stay out of feeds: finding them is the point. A quiet badge
/// (what signing up brings) has neither: nobody is notified, in the inbox or
/// by push, about something they did not do.
async fn notify_unlocked(tx: &mut PgConnection, user_id: Uuid, badge: &BadgeDef) -> AppResult<()> {
    if badge.quiet {
        return Ok(());
    }
    if !badge.title.is_empty() && !badge.secret {
        let shown: bool = sqlx::query_scalar(
            "SELECT COALESCE((SELECT show_on_profile FROM quest_profiles WHERE user_id = $1), true)",
        )
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        let summary = Localized::new(
            format!("称号「{}」をゲット!", badge.title.ja),
            format!("Earned the title \"{}\"", badge.title.en),
        );
        let entry = ActivityEntry::new(user_id, "quest.badge", activity_audience(shown), summary)
            .path(format!("/quest/badges/{}", badge.key))
            .data(serde_json::json!({ "badge_key": badge.key, "emoji": badge.emoji, "name_ja": badge.name.ja, "name_en": badge.name.en, "tier": badge.tier as i32 }));
        outbound::activity(&mut *tx, &entry).await?;
    }
    let (ja, en) = match tier_name(badge.tier) {
        Some((tier_ja, tier_en)) => (
            format!("{} {}（{tier_ja}）", badge.emoji, badge.name.ja),
            format!("{} {} ({tier_en})", badge.emoji, badge.name.en),
        ),
        None => (format!("{} {}", badge.emoji, badge.name.ja), format!("{} {}", badge.emoji, badge.name.en)),
    };
    let request = NotificationRequest::new(
        user_id,
        "quest.badge",
        Localized::new("バッジをゲット!", "Badge unlocked!"),
        Localized::new(ja, en),
        format!("/quest/badges/{}", badge.key),
    )
    .data(serde_json::json!({ "badge_key": badge.key, "family": badge.family.def().key, "tier": badge.tier as i32 }));
    outbound::notify(&mut *tx, &request).await
}

/// Tells a player's devices about a badge they just earned, and records it
/// for analytics. Call after the awarding transaction committed.
pub(crate) async fn announce_unlocked(core: &Core, user_id: Uuid, badge: &BadgeDef) {
    let held = OwnedBadge { badge_key: badge.key.to_owned(), earned_at: now(), times: 1 };
    crate::push(core, user_id, Push::BadgeUnlocked(Box::new(to_proto(badge, Some(&held), None, None)))).await;
    let event = AnalyticsEvent::new("quest.badge")
        .user(Some(user_id))
        .props(serde_json::json!({ "badge": badge.key, "family": badge.family.def().key, "tier": badge.tier as i32 }));
    core.analytics.track(event).await;
}

/// The badges a day earns by itself: an hour of the day, a date, the
/// player's birthday, how many different things they did, a comeback.
/// `previous_active_day` is the last day the player earned anything before
/// this one.
pub(crate) fn day_badges(
    day: NaiveDate,
    counters: &DayCounters,
    birthdate: Option<NaiveDate>,
    previous_active_day: Option<NaiveDate>,
) -> Vec<&'static BadgeDef> {
    let sources = counters.xp.values().filter(|xp| **xp > 0).count() as i32;
    CATALOG
        .iter()
        .filter(|badge| match badge.criterion {
            Criterion::Hours { from, to } => counters.active_between(from, to),
            Criterion::Date { month, day: day_of_month } => day.month() == month && day.day() == day_of_month,
            Criterion::Birthday => birthdate.is_some_and(|born| born.month() == day.month() && born.day() == day.day()),
            Criterion::SourcesInDay(needed) => sources >= needed,
            Criterion::Comeback(days) => {
                previous_active_day.is_some_and(|previous| (day - previous).num_days() >= days as i64)
            }
            _ => false,
        })
        .collect()
}

/// What a player's progress on counted badges is read from.
#[derive(Debug, Default)]
pub(crate) struct Standing {
    pub counters: HashMap<Metric, i64>,
    pub level: i32,
    /// The longest streak the player ever had with anyone.
    pub best_streak: i32,
    /// Running streak lengths, one per friend.
    pub streaks: Vec<i32>,
}

impl Standing {
    /// How far the player is towards a badge, where that is a number.
    pub(crate) fn progress(&self, badge: &BadgeDef) -> Option<i64> {
        match badge.criterion {
            Criterion::Count(metric, _) => Some(self.counters.get(&metric).copied().unwrap_or(0)),
            Criterion::Level(_) => Some(self.level as i64),
            Criterion::StreakDays(_) => Some(self.best_streak as i64),
            Criterion::Streaks { days, .. } => {
                Some(self.streaks.iter().filter(|length| **length >= days).count() as i64)
            }
            _ => None,
        }
    }

    /// Whether the player meets a counted badge's requirement right now.
    fn earns(&self, badge: &BadgeDef) -> bool {
        match badge.criterion {
            Criterion::Count(_, threshold) => self.progress(badge).unwrap_or(0) >= threshold,
            Criterion::Level(level) => self.level >= level,
            Criterion::StreakDays(days) => self.best_streak >= days,
            Criterion::Streaks { count, .. } => self.progress(badge).unwrap_or(0) >= count as i64,
            _ => false,
        }
    }
}

impl Quest {
    /// Where a player stands on everything counted badges count.
    pub(crate) async fn standing(&self, user_id: Uuid) -> AppResult<Standing> {
        let counters: Vec<(String, i64)> =
            sqlx::query_as("SELECT metric, value FROM quest_counters WHERE user_id = $1")
                .bind(user_id)
                .fetch_all(&self.core.db)
                .await?;
        let streaks: Vec<(i32, i32)> =
            sqlx::query_as("SELECT count, best FROM quest_streaks WHERE user_lo = $1 OR user_hi = $1")
                .bind(user_id)
                .fetch_all(&self.core.db)
                .await?;
        Ok(Standing {
            counters: counters.into_iter().filter_map(|(key, value)| Some((Metric::from_key(&key)?, value))).collect(),
            level: self.profile(user_id).await?.level,
            best_streak: streaks.iter().map(|(_, best)| *best).max().unwrap_or(0),
            streaks: streaks.iter().map(|(count, _)| *count).filter(|count| *count > 0).collect(),
        })
    }

    /// Awards the counted badges a player qualifies for but does not own.
    /// Thresholds are normally caught the moment they are crossed; this
    /// catches up after a badge was added to the catalogue (or its
    /// threshold lowered) for players who were already past it.
    pub(crate) async fn catch_up_badges(
        &self,
        user_id: Uuid,
        standing: &Standing,
        owned: &HashMap<String, OwnedBadge>,
    ) -> AppResult<bool> {
        let mut any = false;
        for badge in CATALOG.iter().filter(|badge| !owned.contains_key(badge.key) && standing.earns(badge)) {
            any |= award(&self.core, user_id, badge.key).await?;
        }
        Ok(any)
    }

    /// Rarity of every badge (cached for a minute).
    pub(crate) async fn badge_stats(&self) -> AppResult<Arc<HashMap<String, BadgeStat>>> {
        if let Some(stats) = self.badge_stats.get(&()).await {
            return Ok(stats);
        }
        let rows: Vec<(String, f64, i64)> =
            sqlx::query_as("SELECT badge_key, share, active_players FROM quest_badge_stats")
                .fetch_all(&self.core.db)
                .await?;
        let stats = Arc::new(
            rows.into_iter()
                .map(|(key, share, active_players)| {
                    (key, BadgeStat { share, known: active_players >= MIN_RARITY_PLAYERS })
                })
                .collect::<HashMap<_, _>>(),
        );
        self.badge_stats.insert((), stats.clone()).await;
        Ok(stats)
    }

    /// Recomputes how rare each badge is: the share of active players
    /// (those who earned XP within the last 30 days) who own it.
    pub async fn recompute_rarity(&self) -> AppResult<()> {
        let active_since = chrono::Utc::now().date_naive() - chrono::Duration::days(ACTIVE_DAYS);
        let keys: Vec<&str> = CATALOG.iter().map(|badge| badge.key).collect();
        sqlx::query(
            "WITH active AS (
                 SELECT p.user_id FROM quest_profiles p JOIN users u ON u.id = p.user_id
                 WHERE p.last_active_day >= $1 AND u.deleted_at IS NULL AND u.banned_at IS NULL
             ), total AS (
                 SELECT count(*) AS players FROM active
             ), counts AS (
                 SELECT b.badge_key, count(*) AS owners, count(a.user_id) AS active_owners
                 FROM quest_badges b LEFT JOIN active a ON a.user_id = b.user_id
                 GROUP BY b.badge_key
             )
             INSERT INTO quest_badge_stats (badge_key, owners, active_owners, active_players, share, computed_at)
             SELECT k.key, COALESCE(c.owners, 0), COALESCE(c.active_owners, 0), total.players,
                    CASE WHEN total.players = 0 THEN 0 ELSE COALESCE(c.active_owners, 0)::float8 / total.players END, now()
             FROM UNNEST($2::text[]) AS k (key) CROSS JOIN total LEFT JOIN counts c ON c.badge_key = k.key
             ON CONFLICT (badge_key) DO UPDATE SET
                 owners = EXCLUDED.owners, active_owners = EXCLUDED.active_owners, active_players = EXCLUDED.active_players,
                 share = EXCLUDED.share, computed_at = EXCLUDED.computed_at",
        )
        .bind(active_since)
        .bind(&keys)
        .execute(&self.core.db)
        .await?;
        self.badge_stats.invalidate(&()).await;
        Ok(())
    }

    /// Takes a badge away, also out of the showcase and the title. Returns
    /// whether the player had it.
    pub(crate) async fn take_badge(&self, user_id: Uuid, badge: &BadgeDef) -> AppResult<bool> {
        let mut tx = self.core.db.begin().await?;
        // The profile lock orders this against the player choosing the badge
        // for their showcase or title at the same moment.
        lock_profile(&mut tx, user_id).await?;
        let revoked = sqlx::query("DELETE FROM quest_badges WHERE user_id = $1 AND badge_key = $2")
            .bind(user_id)
            .bind(badge.key)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            > 0;
        if revoked {
            sqlx::query(
                "UPDATE quest_profiles SET showcase = array_remove(showcase, $2), title_badge = CASE WHEN title_badge = $2 THEN '' ELSE title_badge END
                 WHERE user_id = $1",
            )
            .bind(user_id)
            .bind(badge.key)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(revoked)
    }

    /// Stores the player's showcase, provided they own every badge in it.
    pub(crate) async fn store_showcase(&self, user_id: Uuid, keys: &[String]) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        lock_profile(&mut tx, user_id).await?;
        let held: i64 =
            sqlx::query_scalar("SELECT count(*) FROM quest_badges WHERE user_id = $1 AND badge_key = ANY($2)")
                .bind(user_id)
                .bind(keys)
                .fetch_one(&mut *tx)
                .await?;
        ensure!(
            held == keys.len() as i64,
            precondition,
            "BADGE_NOT_OWNED",
            "you do not own every badge of this showcase"
        );
        sqlx::query("UPDATE quest_profiles SET showcase = $2 WHERE user_id = $1")
            .bind(user_id)
            .bind(keys)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Stores the badge whose title the player shows; an empty key removes
    /// the title. The badge must be owned and come with a title.
    pub(crate) async fn store_title(&self, user_id: Uuid, key: &str) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        lock_profile(&mut tx, user_id).await?;
        if !key.is_empty() {
            let held: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM quest_badges WHERE user_id = $1 AND badge_key = $2)")
                    .bind(user_id)
                    .bind(key)
                    .fetch_one(&mut *tx)
                    .await?;
            ensure!(held && catalog::find(key).is_some(), precondition, "BADGE_NOT_OWNED", "you do not own this badge");
            ensure!(
                crate::profile::title_of(key).is_some(),
                precondition,
                "BADGE_HAS_NO_TITLE",
                "this badge does not come with a title"
            );
        }
        sqlx::query("UPDATE quest_profiles SET title_badge = $2 WHERE user_id = $1")
            .bind(user_id)
            .bind(key)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}

/// Players count as active for rarity while they earned XP within this many days.
const ACTIVE_DAYS: i64 = 30;

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn keys(badges: Vec<&'static BadgeDef>) -> Vec<&'static str> {
        badges.into_iter().map(|badge| badge.key).collect()
    }

    #[test]
    fn rarity_classes_follow_the_owner_share() {
        let class = |share| rarity(Some(&BadgeStat { share, known: true }));
        assert_eq!(class(0.9), BadgeRarity::BADGE_RARITY_COMMON);
        assert_eq!(class(0.40), BadgeRarity::BADGE_RARITY_COMMON);
        assert_eq!(class(0.2), BadgeRarity::BADGE_RARITY_UNCOMMON);
        assert_eq!(class(0.05), BadgeRarity::BADGE_RARITY_RARE);
        assert_eq!(class(0.02), BadgeRarity::BADGE_RARITY_EPIC);
        assert_eq!(class(0.0), BadgeRarity::BADGE_RARITY_LEGENDARY);
        assert_eq!(rarity(None), BadgeRarity::BADGE_RARITY_UNSPECIFIED);
        // With too few players a share is not published, whatever it would be.
        let few = BadgeStat { share: 0.5, known: false };
        assert_eq!(rarity(Some(&few)), BadgeRarity::BADGE_RARITY_UNSPECIFIED);
        assert_eq!(to_proto(catalog::find("welcome").unwrap(), None, None, Some(&few)).owner_share, 0.0);
        assert_eq!(masked(catalog::find("birthday").unwrap(), Some(&few)).owner_share, 0.0);
    }

    #[test]
    fn secret_badges_are_masked_until_owned() {
        let badge = catalog::find("early_bird").unwrap();
        let hidden = masked(badge, None);
        assert!(hidden.secret && !hidden.owned);
        // Not even the key says what it is; the stand-in leads back to the badge.
        assert!(hidden.key.starts_with("secret_") && !hidden.key.contains("early"));
        assert_eq!(catalog::find_masked(&hidden.key).map(|found| found.key), Some("early_bird"));
        assert!(hidden.line.is_empty());
        assert!(
            hidden.name.as_option().is_none() && hidden.description.as_option().is_none() && hidden.emoji.is_empty()
        );
        let held = OwnedBadge { badge_key: badge.key.into(), earned_at: now(), times: 1 };
        let shown = to_proto(badge, Some(&held), None, None);
        assert!(shown.owned && shown.secret);
        assert_eq!(shown.name.as_option().unwrap().ja, "早起き");
        assert_eq!(shown.title.as_option().unwrap().en, "Early Bird");
    }

    #[test]
    fn counted_badges_show_progress_towards_their_threshold() {
        let badge = catalog::find("chatter_silver").unwrap();
        let shown = to_proto(badge, None, Some(420), None);
        assert_eq!((shown.progress, shown.target, shown.owned), (420, 1000, false));
        assert_eq!(shown.description.as_option().unwrap().ja, "メッセージを1,000回送る");
        assert_eq!(shown.line, "chatter");
        assert_eq!(shown.tier, BadgeTier::BADGE_TIER_SILVER);
        // Progress never overshoots, and an owned badge is complete.
        assert_eq!(to_proto(badge, None, Some(5000), None).progress, 1000);
        let held = OwnedBadge { badge_key: badge.key.into(), earned_at: now(), times: 1 };
        assert_eq!(to_proto(badge, Some(&held), Some(3), None).progress, 1000);
        // Badges that are not counted have no bar.
        assert_eq!(to_proto(catalog::find("phoenix").unwrap(), None, None, None).target, 0);
    }

    #[test]
    fn a_day_can_earn_date_hour_and_variety_badges() {
        let mut early = DayCounters::default();
        early.hours = 1 << 6;
        assert_eq!(keys(day_badges(date(2026, 11, 22), &early, None, None)), vec!["early_bird"]);
        let mut late = DayCounters::default();
        late.hours = 1 << 7;
        assert!(day_badges(date(2026, 11, 22), &late, None, None).is_empty());
        assert_eq!(keys(day_badges(date(2027, 1, 1), &DayCounters::default(), None, None)), vec!["new_year"]);
        assert_eq!(keys(day_badges(date(2028, 2, 29), &DayCounters::default(), None, None)), vec!["leap_day"]);
        assert_eq!(
            keys(day_badges(date(2026, 11, 22), &DayCounters::default(), Some(date(2010, 11, 22)), None)),
            vec!["birthday"]
        );
        assert!(day_badges(date(2026, 11, 22), &DayCounters::default(), Some(date(2010, 11, 23)), None).is_empty());

        let mut busy = DayCounters::default();
        for source in [
            crate::rules::Source::Chat,
            crate::rules::Source::Friends,
            crate::rules::Source::Map,
            crate::rules::Source::Notes,
            crate::rules::Source::Stamps,
        ] {
            busy.xp.insert(source, 4);
        }
        assert!(day_badges(date(2026, 11, 22), &busy, None, None).is_empty());
        busy.xp.insert(crate::rules::Source::Study, 10);
        assert_eq!(keys(day_badges(date(2026, 11, 22), &busy, None, None)), vec!["all_rounder"]);
    }

    #[test]
    fn coming_back_after_a_month_is_noticed() {
        let day = date(2026, 11, 22);
        assert_eq!(keys(day_badges(day, &DayCounters::default(), None, Some(date(2026, 10, 23)))), vec!["comeback"]);
        assert!(day_badges(day, &DayCounters::default(), None, Some(date(2026, 10, 24))).is_empty());
        assert!(day_badges(day, &DayCounters::default(), None, Some(day)).is_empty());
        assert!(day_badges(day, &DayCounters::default(), None, None).is_empty());
    }

    #[test]
    fn standing_decides_what_a_player_has_earned() {
        let standing = Standing {
            counters: HashMap::from([(Metric::Messages, 1200)]),
            level: 11,
            best_streak: 31,
            streaks: vec![9, 8, 7, 2],
        };
        let earned: Vec<&str> = CATALOG.iter().filter(|badge| standing.earns(badge)).map(|badge| badge.key).collect();
        assert_eq!(
            earned,
            vec![
                "chatter_bronze",
                "chatter_silver",
                "streak_bronze",
                "streak_silver",
                "streak_many_bronze",
                "streak_3",
                "level_bronze",
                "level_silver"
            ]
        );
        assert_eq!(standing.progress(catalog::find("streak_many_gold").unwrap()), Some(3));
        assert_eq!(standing.progress(catalog::find("phoenix").unwrap()), None);
    }
}
