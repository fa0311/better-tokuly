//! Leaderboards among friends, and `LeaderboardService`.
//!
//! A board ranks the caller and their friends — never anybody else — by XP
//! or by their longest running streak: the two things the quest domain
//! owns, and that every player on a board agreed to show (the leaderboard
//! setting).
//!
//! There is deliberately no board for what another domain measures (area
//! explored, time studied). Who may see that is the owning domain's
//! setting, and it can change after the numbers were counted: a board here
//! would keep showing a player who has since hidden their footprints or
//! their studying. Those rankings belong to the domain that knows the
//! setting (school has its own study ranking).
//!
//! Periods are counted in local days: "this week" is Monday to Sunday and
//! "this season" the calendar month, both as of the caller's date. A friend
//! in another time zone is on a different date for part of the day, so
//! around midnight their newest day may already (or not yet) be in the
//! period; nobody's XP is lost or counted twice, the boards just turn over
//! for everyone when the caller's own day does.
use std::time::Duration;

use chrono::NaiveDate;
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::quest::*;
use tokuly_social::graph;

use crate::{
    Quest, clock,
    text::{Text, t},
};

/// What a board ranks by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ranked {
    Xp,
    /// The longest streak a player has running with anyone.
    Streak,
}

struct BoardDef {
    key: &'static str,
    name: Text,
    emoji: &'static str,
    unit: Text,
    ranked: Ranked,
    periods: &'static [LeaderboardPeriod],
}

use LeaderboardPeriod::{
    LEADERBOARD_PERIOD_ALL_TIME as ALL_TIME, LEADERBOARD_PERIOD_SEASON as SEASON, LEADERBOARD_PERIOD_WEEK as WEEK,
};

const BOARDS: &[BoardDef] = &[
    BoardDef {
        key: "xp",
        name: t("XP", "XP"),
        emoji: "⚡",
        unit: t("XP", "XP"),
        ranked: Ranked::Xp,
        periods: &[WEEK, SEASON, ALL_TIME],
    },
    BoardDef {
        key: "streak",
        name: t("ストリーク", "Longest streak"),
        emoji: "🔥",
        unit: t("日", "days"),
        ranked: Ranked::Streak,
        periods: &[ALL_TIME],
    },
];

impl BoardDef {
    fn to_proto(&self) -> Leaderboard {
        Leaderboard {
            key: self.key.to_owned(),
            name: self.name.proto().into(),
            emoji: self.emoji.to_owned(),
            unit: self.unit.proto().into(),
            periods: self.periods.iter().map(|period| (*period).into()).collect(),
            ..Default::default()
        }
    }
}

/// Default and largest number of rows of a board.
const DEFAULT_ROWS: usize = 50;
const MAX_ROWS: usize = 200;

/// Ranks values, highest first: equal values share a rank, and the next
/// rank counts everyone above (1, 1, 3). Players without a value are left out.
pub(crate) fn rank(values: &[(Uuid, i64)]) -> Vec<(Uuid, i64, i32)> {
    let mut sorted: Vec<(Uuid, i64)> = values.iter().copied().filter(|(_, value)| *value > 0).collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut ranked = Vec::with_capacity(sorted.len());
    let mut rank = 0;
    for (index, (user, value)) in sorted.iter().enumerate() {
        if index == 0 || sorted[index - 1].1 != *value {
            rank = index as i32 + 1;
        }
        ranked.push((*user, *value, rank));
    }
    ranked
}

impl Quest {
    /// The caller and those of their friends who appear on leaderboards.
    pub(crate) async fn leaderboard_group(&self, user_id: Uuid) -> AppResult<Vec<Uuid>> {
        let mut candidates = graph::friend_ids(&self.core, user_id).await?;
        candidates.push(user_id);
        Ok(sqlx::query_scalar(
            "SELECT u.id FROM users u LEFT JOIN quest_profiles p ON p.user_id = u.id
             WHERE u.id = ANY($1) AND u.deleted_at IS NULL AND u.banned_at IS NULL AND (u.id = $2 OR COALESCE(p.show_on_leaderboards, true))",
        )
        .bind(&candidates)
        .bind(user_id)
        .fetch_all(&self.core.db)
        .await?)
    }

    /// XP each of `users` earned from `from` on.
    pub(crate) async fn xp_since(&self, users: &[Uuid], from: NaiveDate, to: NaiveDate) -> AppResult<Vec<(Uuid, i64)>> {
        Ok(sqlx::query_as("SELECT user_id, sum(xp)::bigint FROM quest_xp_days WHERE user_id = ANY($1) AND day >= $2 AND day <= $3 GROUP BY user_id").bind(users).bind(from).bind(to).fetch_all(&self.core.db).await?)
    }

    async fn board_values(
        &self,
        board: &BoardDef,
        users: &[Uuid],
        range: Option<(NaiveDate, NaiveDate)>,
    ) -> AppResult<Vec<(Uuid, i64)>> {
        let db = &self.core.db;
        Ok(match (board.ranked, range) {
            (Ranked::Xp, Some((from, to))) => self.xp_since(users, from, to).await?,
            (Ranked::Xp, None) => sqlx::query_as("SELECT user_id, xp FROM quest_profiles WHERE user_id = ANY($1)").bind(users).fetch_all(db).await?,
            // A streak past its deadline is over, whether or not the sweep has been there.
            (Ranked::Streak, _) => {
                sqlx::query_as(
                    "SELECT player, max(count)::bigint FROM (
                         SELECT user_lo AS player, count FROM quest_streaks WHERE user_lo = ANY($1) AND count > 0 AND expires_at > now()
                         UNION ALL
                         SELECT user_hi, count FROM quest_streaks WHERE user_hi = ANY($1) AND count > 0 AND expires_at > now()
                     ) running GROUP BY player",
                )
                .bind(users)
                .fetch_all(db)
                .await?
            }
        })
    }
}

impl LeaderboardService for Quest {
    async fn list_leaderboards(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListLeaderboardsRequest>,
    ) -> ServiceResult<ListLeaderboardsResponse> {
        handle(async {
            ctx.user_id()?;
            let boards = BOARDS.iter().map(BoardDef::to_proto).collect();
            ok(ListLeaderboardsResponse { boards, ..Default::default() })
        })
        .await
    }

    async fn get_leaderboard(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetLeaderboardRequest>,
    ) -> ServiceResult<GetLeaderboardResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            // Each call reads the numbers of every friend.
            self.core.cache.rate_limit(&format!("quest:board:{user_id}"), 120, Duration::from_secs(60)).await?;
            let board = BOARDS
                .iter()
                .find(|board| board.key == request.board)
                .or_not_found("BOARD_NOT_FOUND", "no such leaderboard")?;
            let period = request.period.as_known().filter(|period| board.periods.contains(period));
            let period = match (period, request.period.to_i32()) {
                (Some(period), _) => period,
                // Unset: the board's first period.
                (None, 0) => board.periods[0],
                (None, _) => bail!(invalid, "PERIOD_NOT_SUPPORTED", "this leaderboard does not have that period"),
            };
            let limit = if request.limit <= 0 { DEFAULT_ROWS } else { (request.limit as usize).min(MAX_ROWS) };

            let zone = self.caller_zone(user_id).await?;
            let today = clock::local_date(now(), zone);
            let range = match period {
                WEEK => Some((clock::week_start(today), clock::week_end(today))),
                SEASON => Some((clock::month_start(today), clock::month_end(today))),
                _ => None,
            };
            self.flush_user(user_id).await?;
            let group = self.leaderboard_group(user_id).await?;
            let ranked = rank(&self.board_values(board, &group, range).await?);

            let mut shown: Vec<Uuid> = ranked.iter().take(limit).map(|(user, _, _)| *user).collect();
            if !shown.contains(&user_id) {
                shown.push(user_id);
            }
            let summaries = tokuly_identity::summaries(&self.core, &shown).await?;
            let profiles = self.profiles(&shown).await?;
            let entry = |user: Uuid, value: i64, rank: i32| {
                let profile = profiles.get(&user);
                LeaderboardEntry {
                    rank,
                    user: tokuly_identity::users::pick(&summaries, user).into(),
                    value,
                    me: user == user_id,
                    level: profile.map_or(1, |profile| profile.level),
                    title: profile.and_then(|profile| profile.title()).into(),
                    ..Default::default()
                }
            };
            let entries: Vec<LeaderboardEntry> =
                ranked.iter().take(limit).map(|(user, value, rank)| entry(*user, *value, *rank)).collect();
            let me = ranked
                .iter()
                .find(|(user, _, _)| *user == user_id)
                .map_or_else(|| entry(user_id, 0, 0), |(user, value, rank)| entry(*user, *value, *rank));
            let (period_start, period_end) = range
                .map_or((String::new(), String::new()), |(from, to)| (clock::date_text(from), clock::date_text(to)));
            ok(GetLeaderboardResponse {
                board: board.to_proto().into(),
                period: period.into(),
                period_start,
                period_end,
                entries,
                me: me.into(),
                ranked_count: ranked.len() as i32,
                ..Default::default()
            })
        })
        .await
    }

    async fn get_season(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetSeasonRequest>,
    ) -> ServiceResult<GetSeasonResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            // Like a board: it reads the month of every friend.
            self.core.cache.rate_limit(&format!("quest:board:{user_id}"), 120, Duration::from_secs(60)).await?;
            let zone = self.caller_zone(user_id).await?;
            self.flush_user(user_id).await?;
            ok(GetSeasonResponse {
                season: self.current_season(user_id, zone, now()).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_season_results(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListSeasonResultsRequest>,
    ) -> ServiceResult<ListSeasonResultsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let page = Page::from_view(request.page.as_option(), 60);
            let (seasons, page) = self.season_results_page(user_id, &page).await?;
            ok(ListSeasonResultsResponse { seasons, page: page.into(), ..Default::default() })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn equal_values_share_a_rank_and_zero_is_unranked() {
        let ids: Vec<Uuid> = (0..5).map(|_| Uuid::now_v7()).collect();
        let ranked = rank(&[(ids[0], 50), (ids[1], 120), (ids[2], 50), (ids[3], 0), (ids[4], 7)]);
        let ranks: Vec<(i64, i32)> = ranked.iter().map(|(_, value, rank)| (*value, *rank)).collect();
        assert_eq!(ranks, vec![(120, 1), (50, 2), (50, 2), (7, 4)]);
        assert!(!ranked.iter().any(|(user, _, _)| *user == ids[3]));
        assert!(rank(&[]).is_empty());
    }

    #[test]
    fn boards_have_unique_keys_and_at_least_one_period() {
        let mut keys = HashSet::new();
        for board in BOARDS {
            assert!(keys.insert(board.key));
            assert!(!board.periods.is_empty());
            assert!(!board.name.ja.is_empty() && !board.unit.en.is_empty());
        }
        for key in ["xp", "streak"] {
            assert!(keys.contains(key), "board {key} is missing");
        }
    }
}
