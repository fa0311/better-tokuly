//! Study time added up: per day, per subject, per friend.
//!
//! Every total is a sum over `school_study_segments`. "Per day" cuts the
//! segments at the local midnights of the time zone asked for, so a session
//! across midnight counts towards both days, and the same log reads
//! differently — correctly — after someone moves to another time zone.
use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{Duration, NaiveDate};
use chrono_tz::Tz;
use sqlx::PgConnection;
use tokuly_core::prelude::*;
use tokuly_proto::school::{
    DayTotal, GetStudyHeatmapResponse, GetStudyRankingResponse, GetStudySummaryResponse, RankingEntry, StatsPeriod,
    StudyProgress, SubjectTotal,
};
use tokuly_social::graph;

use crate::{
    School, local,
    settings::{self, SettingsRow},
    subjects,
};

/// No segment is longer than a day (sessions and manual entries are capped),
/// so a window only needs the segments that started within a day before it.
const SEGMENT_REACH: &str = "interval '25 hours'";
/// Study time below this does not make a day count towards a streak.
const STREAK_MIN_SECONDS: i64 = 60;
const MAX_HEATMAP_DAYS: i32 = 371;
const RANKING_SIZE: usize = 100;

/// Seconds studied on each local day between two dates (inclusive). Days
/// without study are absent.
pub(crate) async fn day_totals(
    conn: &mut PgConnection,
    user_id: Uuid,
    tz: Tz,
    from: NaiveDate,
    to: NaiveDate,
) -> AppResult<BTreeMap<NaiveDate, i64>> {
    let rows: Vec<(NaiveDate, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT day::date, floor(sum(seconds))::bigint
         FROM (
             SELECT d AS day,
                    EXTRACT(EPOCH FROM LEAST(s.ended_at, (d + interval '1 day') AT TIME ZONE $2) - GREATEST(s.started_at, d AT TIME ZONE $2)) AS seconds
             FROM school_study_segments s
             CROSS JOIN LATERAL generate_series(
                 date_trunc('day', s.started_at AT TIME ZONE $2),
                 date_trunc('day', (s.ended_at - interval '1 microsecond') AT TIME ZONE $2),
                 interval '1 day') AS d
             WHERE s.user_id = $1 AND s.started_at >= $3::timestamptz - {SEGMENT_REACH} AND s.started_at < $4
         ) per_day
         WHERE day::date BETWEEN $5 AND $6
         GROUP BY day
         HAVING floor(sum(seconds)) > 0"
    )))
    .bind(user_id)
    .bind(tz.name())
    .bind(local::day_start(from, tz))
    .bind(local::day_start(to + Duration::days(1), tz))
    .bind(from)
    .bind(to)
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().collect())
}

/// Seconds each of `user_ids` studied between two instants. Users without
/// study time are absent.
pub(crate) async fn window_totals(
    conn: &mut PgConnection,
    user_ids: &[Uuid],
    from: Time,
    to: Time,
) -> AppResult<HashMap<Uuid, i64>> {
    let rows: Vec<(Uuid, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT user_id, floor(sum(EXTRACT(EPOCH FROM LEAST(ended_at, $3) - GREATEST(started_at, $2))))::bigint
         FROM school_study_segments
         WHERE user_id = ANY($1) AND started_at >= $2::timestamptz - {SEGMENT_REACH} AND started_at < $3 AND ended_at > $2
         GROUP BY user_id"
    )))
    .bind(user_ids)
    .bind(from)
    .bind(to)
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().collect())
}

/// Seconds one user studied each subject between two instants (`None` =
/// without a subject), most studied first.
async fn subject_totals(
    conn: &mut PgConnection,
    user_id: Uuid,
    from: Time,
    to: Time,
) -> AppResult<Vec<(Option<Uuid>, i64)>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT ss.subject_id, floor(sum(EXTRACT(EPOCH FROM LEAST(s.ended_at, $3) - GREATEST(s.started_at, $2))))::bigint AS seconds
         FROM school_study_segments s JOIN school_study_sessions ss ON ss.id = s.session_id
         WHERE s.user_id = $1 AND s.started_at >= $2::timestamptz - {SEGMENT_REACH} AND s.started_at < $3 AND s.ended_at > $2
         GROUP BY ss.subject_id
         HAVING floor(sum(EXTRACT(EPOCH FROM LEAST(s.ended_at, $3) - GREATEST(s.started_at, $2)))) > 0
         ORDER BY seconds DESC, ss.subject_id"
    )))
    .bind(user_id)
    .bind(from)
    .bind(to)
    .fetch_all(conn)
    .await?)
}

/// The streak ending today (or yesterday, when today does not count yet)
/// and the longest one, over the days that count.
pub(crate) fn streaks(days: &BTreeSet<NaiveDate>, today: NaiveDate) -> (i32, i32) {
    let mut longest = 0;
    let mut run = 0;
    let mut previous: Option<NaiveDate> = None;
    for day in days {
        run = if previous == Some(*day - Duration::days(1)) { run + 1 } else { 1 };
        longest = longest.max(run);
        previous = Some(*day);
    }
    let mut current = 0;
    let mut day = if days.contains(&today) { today } else { today - Duration::days(1) };
    while days.contains(&day) {
        current += 1;
        day -= Duration::days(1);
    }
    (current, longest)
}

/// Today against the goal, the week so far, and the streak.
pub(crate) async fn progress(
    conn: &mut PgConnection,
    user_id: Uuid,
    tz: Tz,
    settings: &SettingsRow,
    at: Time,
) -> AppResult<StudyProgress> {
    let today = local::local_date(at, tz);
    let earliest = NaiveDate::from_ymd_opt(2000, 1, 1).expect("a valid date");
    let totals = day_totals(conn, user_id, tz, earliest, today).await?;
    let goal_seconds = settings.daily_goal_minutes as i64 * 60;
    let needed = if goal_seconds > 0 { goal_seconds } else { STREAK_MIN_SECONDS };
    let counting: BTreeSet<NaiveDate> =
        totals.iter().filter(|(_, seconds)| **seconds >= needed).map(|(day, _)| *day).collect();
    let (streak_days, longest_streak_days) = streaks(&counting, today);
    let today_seconds = totals.get(&today).copied().unwrap_or(0);
    Ok(StudyProgress {
        date: local::fmt_date(today),
        today_seconds: today_seconds as i32,
        goal_minutes: settings.daily_goal_minutes,
        goal_reached: goal_seconds > 0 && today_seconds >= goal_seconds,
        streak_days,
        longest_streak_days,
        week_seconds: totals.range(local::week_start(today)..=today).map(|(_, seconds)| *seconds).sum::<i64>() as i32,
        total_seconds: totals.values().sum(),
        ..Default::default()
    })
}

fn day_total(date: NaiveDate, seconds: i64) -> DayTotal {
    DayTotal { date: local::fmt_date(date), seconds: seconds as i32, ..Default::default() }
}

/// First and last day of the period around `anchor`, and of the one before it.
fn period_range(period: StatsPeriod, anchor: NaiveDate) -> AppResult<((NaiveDate, NaiveDate), (NaiveDate, NaiveDate))> {
    let day = Duration::days(1);
    Ok(match period {
        StatsPeriod::STATS_PERIOD_DAY => ((anchor, anchor), (anchor - day, anchor - day)),
        StatsPeriod::STATS_PERIOD_WEEK => {
            let start = local::week_start(anchor);
            ((start, start + Duration::days(6)), (start - Duration::days(7), start - day))
        }
        StatsPeriod::STATS_PERIOD_MONTH => {
            let start = local::month_start(anchor);
            ((start, local::next_month_start(anchor) - day), (local::month_start(start - day), start - day))
        }
        StatsPeriod::STATS_PERIOD_UNSPECIFIED => {
            return Err(AppError::invalid("STATS_PERIOD_REQUIRED", "say which period: day, week or month"));
        }
    })
}

impl School {
    pub(crate) async fn study_summary(
        &self,
        user_id: Uuid,
        period: StatsPeriod,
        anchor_date: &str,
    ) -> AppResult<GetStudySummaryResponse> {
        let core = &self.core;
        let tz = local::time_zone(core, user_id).await?;
        let anchor = local::parse_opt_date(anchor_date)?.unwrap_or_else(|| local::local_date(self.now(), tz));
        let ((start, end), (previous_start, previous_end)) = period_range(period, anchor)?;
        let window = |first: NaiveDate, last: NaiveDate| {
            (local::day_start(first, tz), local::day_start(last + Duration::days(1), tz))
        };

        let mut conn = core.db.acquire().await?;
        let totals = day_totals(&mut conn, user_id, tz, start, end).await?;
        let (from, to) = window(start, end);
        let by_subject = subject_totals(&mut conn, user_id, from, to).await?;
        let (previous_from, previous_to) = window(previous_start, previous_end);
        let previous = window_totals(&mut conn, &[user_id], previous_from, previous_to).await?;
        drop(conn);

        let subject_ids: Vec<Uuid> = by_subject.iter().filter_map(|(id, _)| *id).collect();
        let subjects = subjects::by_ids(core, &subject_ids).await?;
        Ok(GetStudySummaryResponse {
            start_date: local::fmt_date(start),
            end_date: local::fmt_date(end),
            total_seconds: totals.values().sum::<i64>() as i32,
            previous_total_seconds: previous.get(&user_id).copied().unwrap_or(0) as i32,
            days: crate::timetable::dates(start, end)
                .map(|date| day_total(date, totals.get(&date).copied().unwrap_or(0)))
                .collect(),
            subjects: by_subject
                .into_iter()
                .map(|(id, seconds)| SubjectTotal {
                    subject: id.and_then(|id| subjects.get(&id)).map(|subject| subject.to_proto()).into(),
                    seconds: seconds as i32,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })
    }

    pub(crate) async fn study_progress(&self, user_id: Uuid, at: Time) -> AppResult<StudyProgress> {
        let mut conn = self.core.db.acquire().await?;
        let settings = settings::of_on(&mut conn, user_id).await?;
        let tz = local::time_zone_on(&mut conn, user_id).await?;
        progress(&mut conn, user_id, tz, &settings, at).await
    }

    /// A year (or less) of study time per day: the caller's own, or a
    /// friend's who shows their studying to friends.
    pub(crate) async fn study_heatmap(
        &self,
        viewer: Uuid,
        owner: Uuid,
        end_date: &str,
        days: i32,
    ) -> AppResult<GetStudyHeatmapResponse> {
        let core = &self.core;
        if owner != viewer {
            // Not a friend, hidden by its owner, or a suspended account: one answer for all.
            let shared = graph::are_friends(core, owner, viewer).await?
                && settings::of(core, owner).await?.shows_studying()
                && !tokuly_identity::users::filter_active(core, &[owner]).await?.is_empty();
            ensure!(shared, not_found, "STUDY_NOT_SHARED", "this study log is not shared with you");
        }
        let days = if days == 0 { 365 } else { days };
        ensure!(
            (1..=MAX_HEATMAP_DAYS).contains(&days),
            invalid,
            "DATE_RANGE_INVALID",
            "a heatmap covers 1 to {MAX_HEATMAP_DAYS} days"
        );
        // The owner's days, as the owner counts them.
        let tz = local::time_zone(core, owner).await?;
        let end = local::parse_opt_date(end_date)?.unwrap_or_else(|| local::local_date(self.now(), tz));
        let start = end - Duration::days(days as i64 - 1);
        let totals = day_totals(&mut *core.db.acquire().await?, owner, tz, start, end).await?;
        Ok(GetStudyHeatmapResponse {
            start_date: local::fmt_date(start),
            end_date: local::fmt_date(end),
            max_seconds: totals.values().copied().max().unwrap_or(0) as i32,
            total_seconds: totals.values().sum::<i64>() as i32,
            days: totals.into_iter().map(|(date, seconds)| day_total(date, seconds)).collect(),
            ..Default::default()
        })
    }

    /// The week's study time among the caller and the friends who joined the
    /// ranking. The week is the caller's (their time zone).
    pub(crate) async fn study_ranking(&self, viewer: Uuid, anchor_date: &str) -> AppResult<GetStudyRankingResponse> {
        let core = &self.core;
        let tz = local::time_zone(core, viewer).await?;
        let anchor = local::parse_opt_date(anchor_date)?.unwrap_or_else(|| local::local_date(self.now(), tz));
        let start = local::week_start(anchor);
        let end = start + Duration::days(6);
        let mut response = GetStudyRankingResponse {
            start_date: local::fmt_date(start),
            end_date: local::fmt_date(end),
            ..Default::default()
        };
        if !settings::of(core, viewer).await?.join_ranking {
            return Ok(response);
        }
        response.joined = true;

        // Friends who joined — and whose account is around: a suspended one
        // drops out the way one that left the ranking does.
        let friends = tokuly_identity::users::filter_active(core, &graph::friend_ids(core, viewer).await?).await?;
        let joined = settings::of_many(core, &friends).await?;
        let mut participants: Vec<Uuid> =
            friends.into_iter().filter(|friend| joined.get(friend).is_some_and(|s| s.join_ranking)).collect();
        participants.push(viewer);
        let totals = window_totals(
            &mut *core.db.acquire().await?,
            &participants,
            local::day_start(start, tz),
            local::day_start(end + Duration::days(1), tz),
        )
        .await?;
        let users = tokuly_identity::summaries(core, &participants).await?;

        let mut ranked: Vec<(Uuid, i64)> = participants
            .into_iter()
            .filter(|id| users.get(id).is_some_and(|user| !user.deleted))
            .map(|id| (id, totals.get(&id).copied().unwrap_or(0)))
            .collect();
        ranked.sort_by_key(|(id, seconds)| (std::cmp::Reverse(*seconds), *id));
        let mut rank = 0;
        let mut previous_seconds = None;
        for (index, (id, seconds)) in ranked.into_iter().enumerate() {
            // Equal times share a rank; the next one counts everyone above it.
            if previous_seconds != Some(seconds) {
                rank = index as i32 + 1;
                previous_seconds = Some(seconds);
            }
            let me = id == viewer;
            if index < RANKING_SIZE || me {
                response.entries.push(RankingEntry {
                    rank,
                    user: users.get(&id).cloned().into(),
                    seconds: seconds as i32,
                    me,
                    ..Default::default()
                });
            }
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).unwrap()
    }

    fn days(list: &[(u32, u32)]) -> BTreeSet<NaiveDate> {
        list.iter().map(|(month, day)| date(*month, *day)).collect()
    }

    #[test]
    fn a_streak_runs_until_today_or_yesterday() {
        let studied = days(&[(4, 1), (4, 2), (4, 3), (4, 5), (4, 6)]);
        assert_eq!(streaks(&studied, date(4, 6)), (2, 3), "today counts");
        assert_eq!(streaks(&studied, date(4, 7)), (2, 3), "today has not happened yet: the streak stands");
        assert_eq!(streaks(&studied, date(4, 8)), (0, 3), "a day was missed");
        assert_eq!(streaks(&studied, date(4, 4)), (3, 3));
        assert_eq!(streaks(&BTreeSet::new(), date(4, 4)), (0, 0));
        // Across a month boundary.
        assert_eq!(streaks(&days(&[(3, 30), (3, 31), (4, 1)]), date(4, 1)), (3, 3));
    }

    #[test]
    fn periods_span_the_day_the_week_and_the_month() {
        let anchor = date(4, 8); // a Wednesday
        assert_eq!(
            period_range(StatsPeriod::STATS_PERIOD_DAY, anchor).unwrap(),
            ((date(4, 8), date(4, 8)), (date(4, 7), date(4, 7)))
        );
        assert_eq!(
            period_range(StatsPeriod::STATS_PERIOD_WEEK, anchor).unwrap(),
            ((date(4, 6), date(4, 12)), (date(3, 30), date(4, 5)))
        );
        assert_eq!(
            period_range(StatsPeriod::STATS_PERIOD_MONTH, anchor).unwrap(),
            ((date(4, 1), date(4, 30)), (date(3, 1), date(3, 31)))
        );
        assert_eq!(
            period_range(StatsPeriod::STATS_PERIOD_UNSPECIFIED, anchor).unwrap_err().reason,
            "STATS_PERIOD_REQUIRED"
        );
    }
}
