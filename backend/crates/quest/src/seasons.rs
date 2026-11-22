//! Seasons: one calendar month of XP among friends.
//!
//! A player's season XP is what they earned on the local days of the month,
//! by their own calendar. While the month runs, [`Quest::current_season`]
//! ranks the caller among their friends live. When it is over everywhere on
//! Earth (12 hours after it ended in UTC), [`Quest::finalize_due_seasons`]
//! takes the final snapshot: every participant's XP, their rank among
//! themselves and their friends who took part, and the season badge for the
//! top third of each such group — rounded up, so a group of two has a
//! winner and a group of four has two. Nothing is reset: the next month
//! simply counts its own days.
use chrono::{DateTime, Datelike as _, NaiveDate, Utc};
use chrono_tz::Tz;
use tokuly_core::{
    analytics::AnalyticsEvent,
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::{common::PageInfo, quest as pb};
use tokuly_social::graph;

use crate::{
    Quest,
    badges::{self, catalog},
    clock,
    earnings::{Earned, Effects},
    leaderboards::rank,
    metrics::Metric,
    profile::lock_profile,
    text::localized_text,
};

/// Smallest group (the player included) in which a season has winners.
const MIN_GROUP: i32 = 2;
/// Smallest group in which finishing first makes a champion.
const CHAMPION_GROUP: i32 = 3;
/// Finished months looked back at for seasons that were never finalized
/// (the worker was down over the turn of a month).
const CATCH_UP_MONTHS: u32 = 3;
/// Participants ranked per round.
const RANK_BATCH: i64 = 200;

const ENGLISH_MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Ranks up to this one win the badge: the top third, rounded up. Zero
/// while the group is too small to have a contest.
pub(crate) fn badge_rank(group_size: i32) -> i32 {
    if group_size < MIN_GROUP { 0 } else { (group_size + 2) / 3 }
}

fn season_name(month: NaiveDate) -> tokuly_proto::common::LocalizedText {
    localized_text(
        format!("{}年{}月シーズン", month.year(), month.month()),
        format!("{} {} season", ENGLISH_MONTHS[month.month0() as usize], month.year()),
    )
}

/// The month before the one `month` starts.
fn previous_month(month: NaiveDate) -> NaiveDate {
    clock::month_start(clock::previous_day(clock::month_start(month)))
}

/// The latest month that is over everywhere at `now`.
pub(crate) fn last_finished_month(now: DateTime<Utc>) -> NaiveDate {
    let mut month = clock::month_start(now.date_naive());
    while clock::month_over_everywhere(month) > now {
        month = previous_month(month);
    }
    month
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ResultRow {
    month: NaiveDate,
    xp: i64,
    rank: i32,
    group_size: i32,
    badge_won: bool,
}

impl Quest {
    /// A season as the API shows it to one player.
    fn season_proto(
        &self,
        month: NaiveDate,
        xp: i64,
        rank: i32,
        group_size: i32,
        finished: bool,
        badge_won: bool,
        badge: pb::Badge,
        zone: Option<Tz>,
    ) -> pb::Season {
        pb::Season {
            id: clock::month_id(month),
            name: season_name(month).into(),
            starts_on: clock::date_text(month),
            ends_on: clock::date_text(clock::month_end(month)),
            xp,
            rank,
            group_size,
            badge_rank: badge_rank(group_size),
            badge: badge.into(),
            r#final: finished,
            badge_won,
            ends_at: opt_ts(zone.map(|zone| clock::end_of_day(clock::month_end(month), zone))),
            ..Default::default()
        }
    }

    /// The season badge of a month, as `user_id` holds it (or not).
    async fn season_badge_for(&self, user_id: Uuid, month: NaiveDate) -> AppResult<pb::Badge> {
        let badge = catalog::season_badge(month.month());
        let held: Option<badges::OwnedBadge> = sqlx::query_as(
            "SELECT badge_key, earned_at, times FROM quest_badges WHERE user_id = $1 AND badge_key = $2",
        )
        .bind(user_id)
        .bind(badge.key)
        .fetch_optional(&self.core.db)
        .await?;
        Ok(badges::to_proto(badge, held.as_ref(), None, self.badge_stats().await?.get(badge.key)))
    }

    /// The running season: the caller's XP this month and their rank among
    /// the friends who earned any.
    pub(crate) async fn current_season(&self, user_id: Uuid, zone: Tz, now: DateTime<Utc>) -> AppResult<pb::Season> {
        let month = clock::month_start(clock::local_date(now, zone));
        let group = self.leaderboard_group(user_id).await?;
        let ranked = rank(&self.xp_since(&group, month, clock::month_end(month)).await?);
        let (xp, my_rank) =
            ranked.iter().find(|(user, _, _)| *user == user_id).map_or((0, 0), |(_, xp, rank)| (*xp, *rank));
        let badge = self.season_badge_for(user_id, month).await?;
        Ok(self.season_proto(month, xp, my_rank, ranked.len() as i32, false, false, badge, Some(zone)))
    }

    /// A page of the caller's results in finished seasons, newest first.
    pub(crate) async fn season_results_page(
        &self,
        user_id: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<pb::Season>, PageInfo)> {
        let before: Option<NaiveDate> = page.decode()?;
        let mut rows: Vec<ResultRow> = sqlx::query_as(
            "SELECT r.month, r.xp, r.rank, r.group_size, r.badge_won
             FROM quest_season_results r JOIN quest_seasons s ON s.month = r.month
             WHERE r.user_id = $1 AND s.finalized_at IS NOT NULL AND ($2::date IS NULL OR r.month < $2)
             ORDER BY r.month DESC LIMIT $3",
        )
        .bind(user_id)
        .bind(before)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| row.month);
        let mut seasons = Vec::with_capacity(rows.len());
        for row in rows {
            let badge = self.season_badge_for(user_id, row.month).await?;
            seasons.push(self.season_proto(
                row.month,
                row.xp,
                row.rank,
                row.group_size,
                true,
                row.badge_won,
                badge,
                None,
            ));
        }
        Ok((seasons, info))
    }

    /// Takes the final snapshot of every month that is over everywhere and
    /// has none yet. Returns the months finalized by this call. Safe to run
    /// in several processes and to interrupt: it picks up where it stopped.
    pub async fn finalize_due_seasons(&self) -> AppResult<Vec<NaiveDate>> {
        let mut month = last_finished_month(Utc::now());
        let mut due = Vec::new();
        for _ in 0..CATCH_UP_MONTHS {
            let finalized: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM quest_seasons WHERE month = $1 AND finalized_at IS NOT NULL)",
            )
            .bind(month)
            .fetch_one(&self.core.db)
            .await?;
            if !finalized {
                due.push(month);
            }
            month = previous_month(month);
        }
        let mut finalized = Vec::new();
        // Oldest first, so "seasons won" counts in order.
        for month in due.into_iter().rev() {
            self.finalize_season(month).await?;
            finalized.push(month);
        }
        Ok(finalized)
    }

    /// Finalizes one season. Safe to run in several processes at once: each
    /// participant is settled by whichever gets to them first (the update
    /// that records a rank decides), so nothing is awarded twice — and no
    /// lock, and no connection, is held across the work.
    pub async fn finalize_season(&self, month: NaiveDate) -> AppResult<()> {
        let db = &self.core.db;
        sqlx::query("INSERT INTO quest_seasons (month) VALUES ($1) ON CONFLICT (month) DO NOTHING")
            .bind(month)
            .execute(db)
            .await?;
        // The snapshot: everyone who earned XP on a day of the month. Rank 0
        // marks a participant whose rank is still to be computed.
        sqlx::query(
            "INSERT INTO quest_season_results (month, user_id, xp, rank, group_size)
             SELECT $1, x.user_id, sum(x.xp), 0, 0
             FROM quest_xp_days x JOIN users u ON u.id = x.user_id
             WHERE x.day >= $1 AND x.day <= $2 AND u.deleted_at IS NULL AND u.banned_at IS NULL
             GROUP BY x.user_id HAVING sum(x.xp) > 0
             ON CONFLICT (month, user_id) DO NOTHING",
        )
        .bind(month)
        .bind(clock::month_end(month))
        .execute(db)
        .await?;
        loop {
            let pending: Vec<(Uuid, i64)> = sqlx::query_as(
                "SELECT user_id, xp FROM quest_season_results WHERE month = $1 AND rank = 0 ORDER BY user_id LIMIT $2",
            )
            .bind(month)
            .bind(RANK_BATCH)
            .fetch_all(db)
            .await?;
            if pending.is_empty() {
                break;
            }
            for (user_id, xp) in pending {
                self.settle_season_result(month, user_id, xp).await?;
            }
        }
        sqlx::query("UPDATE quest_seasons SET finalized_at = now(), participants = (SELECT count(*) FROM quest_season_results WHERE month = $1) WHERE month = $1").bind(month).execute(db).await?;
        tracing::info!(season = %clock::month_id(month), "quest: season finalized");
        Ok(())
    }

    /// Ranks one participant among their friends who took part and hands
    /// out what the rank earns.
    async fn settle_season_result(&self, month: NaiveDate, user_id: Uuid, xp: i64) -> AppResult<()> {
        let db = &self.core.db;
        let friends = graph::friend_ids(&self.core, user_id).await?;
        // Friends who took part and show themselves on leaderboards.
        let rivals: Vec<i64> = sqlx::query_scalar(
            "SELECT r.xp FROM quest_season_results r LEFT JOIN quest_profiles p ON p.user_id = r.user_id
             WHERE r.month = $1 AND r.user_id = ANY($2) AND COALESCE(p.show_on_leaderboards, true)",
        )
        .bind(month)
        .bind(&friends)
        .fetch_all(db)
        .await?;
        let group_size = rivals.len() as i32 + 1;
        let rank = rivals.iter().filter(|rival| **rival > xp).count() as i32 + 1;
        let won = rank <= badge_rank(group_size);
        let champion = won && rank == 1 && group_size >= CHAMPION_GROUP;
        let zone = self.zone_of(user_id).await?;
        let today = clock::local_date(now(), zone.unwrap_or(clock::DEFAULT_ZONE));
        let season = clock::month_id(month);

        let mut tx = db.begin().await?;
        let mut profile = lock_profile(&mut tx, user_id).await?;
        let mut effects = Effects::starting_from(&profile);
        let first = sqlx::query("UPDATE quest_season_results SET rank = $3, group_size = $4, badge_won = $5 WHERE month = $1 AND user_id = $2 AND rank = 0")
            .bind(month)
            .bind(user_id)
            .bind(rank)
            .bind(group_size)
            .bind(won)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            > 0;
        if !first {
            return Ok(());
        }
        let badge = catalog::season_badge(month.month());
        if won {
            let data = serde_json::json!({ "season": season, "rank": rank, "group_size": group_size });
            if badges::award_again_tx(&mut tx, user_id, badge, data.clone()).await? {
                effects.badges.push(badge);
            }
            self.apply(
                &mut tx,
                &mut profile,
                today,
                &Earned { xp: Vec::new(), metrics: vec![(Metric::SeasonsWon, 1)] },
                &mut effects,
            )
            .await?;
            if champion
                && let Some(badge) = catalog::find(catalog::SEASON_CHAMPION)
                && badges::award_tx(&mut tx, user_id, badge, data).await?
            {
                effects.badges.push(badge);
            }
            let months_won: i64 =
                sqlx::query_scalar("SELECT count(*) FROM quest_badges WHERE user_id = $1 AND badge_key = ANY($2)")
                    .bind(user_id)
                    .bind(&catalog::SEASON_MONTHS[..])
                    .fetch_one(&mut *tx)
                    .await?;
            if months_won == catalog::SEASON_MONTHS.len() as i64
                && let Some(badge) = catalog::find(catalog::FULL_YEAR)
                && badges::award_tx(&mut tx, user_id, badge, serde_json::Value::Null).await?
            {
                effects.badges.push(badge);
            }
        }
        // A result is only news to somebody who had rivals.
        if group_size >= MIN_GROUP && zone.is_some() {
            let name = season_name(month);
            let body = if won {
                Localized::new(
                    format!("友達{group_size}人中{rank}位! {} {}をゲット!", badge.emoji, badge.name.ja),
                    format!("#{rank} of {group_size} friends! You won {} {}!", badge.emoji, badge.name.en),
                )
            } else {
                Localized::new(
                    format!("友達{group_size}人中{rank}位でした。今月もたのしもう!"),
                    format!("You finished #{rank} of {group_size} friends. On to the next one!"),
                )
            };
            let mut notice = NotificationRequest::new(
                user_id,
                "quest.season",
                Localized::new(format!("{}が終わりました!", name.ja), format!("The {} is over!", name.en)),
                body,
                "/quest/seasons",
            )
            .collapse(format!("quest.season:{season}"))
            .data(serde_json::json!({ "season": season, "rank": rank, "group_size": group_size, "badge_won": won }));
            if !won {
                notice = notice.inbox_only();
            }
            outbound::notify(&mut *tx, &notice).await?;
        }
        tx.commit().await?;
        if won {
            let event = AnalyticsEvent::new("quest.season.badge")
                .user(Some(user_id))
                .value(rank as f64)
                .props(serde_json::json!({ "season": season, "group_size": group_size, "champion": champion }));
            self.core.analytics.track(event).await;
        }
        self.announce(user_id, effects).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn the_top_third_wins_rounded_up_so_small_groups_have_winners() {
        assert_eq!(badge_rank(1), 0);
        assert_eq!(badge_rank(2), 1);
        assert_eq!(badge_rank(3), 1);
        assert_eq!(badge_rank(4), 2);
        assert_eq!(badge_rank(6), 2);
        assert_eq!(badge_rank(7), 3);
        assert_eq!(badge_rank(30), 10);
        for size in 2..200 {
            let winners = badge_rank(size);
            assert!(winners >= 1 && winners * 3 >= size && (winners - 1) * 3 < size, "group of {size}");
        }
    }

    #[test]
    fn a_month_is_finished_once_it_is_over_everywhere() {
        // Noon UTC on the first: the last time zone has just left November.
        assert_eq!(last_finished_month(Utc.with_ymd_and_hms(2026, 12, 1, 12, 0, 0).unwrap()), date(2026, 11, 1));
        // An hour earlier somebody is still in it.
        assert_eq!(last_finished_month(Utc.with_ymd_and_hms(2026, 12, 1, 11, 0, 0).unwrap()), date(2026, 10, 1));
        assert_eq!(last_finished_month(Utc.with_ymd_and_hms(2026, 12, 20, 0, 0, 0).unwrap()), date(2026, 11, 1));
        assert_eq!(last_finished_month(Utc.with_ymd_and_hms(2027, 1, 1, 12, 0, 0).unwrap()), date(2026, 12, 1));
        assert_eq!(previous_month(date(2027, 1, 1)), date(2026, 12, 1));
    }

    #[test]
    fn seasons_are_named_after_their_month() {
        let name = season_name(date(2026, 11, 1));
        assert_eq!((name.ja.as_str(), name.en.as_str()), ("2026年11月シーズン", "November 2026 season"));
    }
}
