//! Calendar days and time zones.
//!
//! A plan "on the 24th", a countdown to a festival and an anniversary are
//! days, not instants: which instant they mean depends on a time zone. The
//! helpers here convert between the two, and answer "what day is it for this
//! user".
use std::collections::HashMap;

use chrono::{Datelike as _, Duration, NaiveDate, NaiveDateTime, TimeZone as _, Utc};
use chrono_tz::Tz;
use tokuly_core::prelude::*;

/// Used for accounts whose stored time zone is not a zone we know.
pub(crate) const FALLBACK_ZONE: Tz = chrono_tz::Asia::Tokyo;

/// A stored zone name as a time zone.
pub(crate) fn zone(name: &str) -> Tz {
    name.parse().unwrap_or(FALLBACK_ZONE)
}

/// The time zone a user lives in (from their account).
pub(crate) async fn zone_of(core: &Core, user_id: Uuid) -> AppResult<Tz> {
    let name: Option<String> =
        sqlx::query_scalar("SELECT time_zone FROM users WHERE id = $1").bind(user_id).fetch_optional(&core.db).await?;
    Ok(name.as_deref().map(zone).unwrap_or(FALLBACK_ZONE))
}

/// [`zone_of`] for several users; unknown users are absent.
pub(crate) async fn zones_of(core: &Core, user_ids: &[Uuid]) -> AppResult<HashMap<Uuid, Tz>> {
    let rows: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT id, time_zone FROM users WHERE id = ANY($1)").bind(user_ids).fetch_all(&core.db).await?;
    Ok(rows.into_iter().map(|(id, name)| (id, zone(&name))).collect())
}

/// The instant at which the clocks in `zone` show `hour` o'clock on `date`.
/// When that time is skipped (clocks jumping forward) the hour after is
/// taken; when it happens twice, the first time.
pub(crate) fn local_instant(date: NaiveDate, hour: u32, zone: Tz) -> Time {
    let wall_clock = date.and_hms_opt(hour.min(23), 0, 0).expect("an hour of the day");
    let resolve =
        |time: NaiveDateTime| zone.from_local_datetime(&time).earliest().map(|local| local.with_timezone(&Utc));
    resolve(wall_clock)
        .or_else(|| resolve(wall_clock + Duration::hours(1)))
        .unwrap_or_else(|| Utc.from_utc_datetime(&wall_clock))
}

/// The instant `date` begins in `zone`.
pub(crate) fn day_start(date: NaiveDate, zone: Tz) -> Time {
    local_instant(date, 0, zone)
}

/// The instant `date` ends in `zone` (the beginning of the next day).
pub(crate) fn day_end(date: NaiveDate, zone: Tz) -> Time {
    day_start(next_day(date), zone)
}

pub(crate) fn next_day(date: NaiveDate) -> NaiveDate {
    date.succ_opt().unwrap_or(date)
}

/// The calendar day it is in `zone` at `at`.
pub(crate) fn day_in(zone: Tz, at: Time) -> NaiveDate {
    at.with_timezone(&zone).date_naive()
}

/// Parses `YYYY-MM-DD`. Years far from the present are rejected so that day
/// arithmetic never overflows.
pub(crate) fn parse_date(input: &str) -> Option<NaiveDate> {
    let date = NaiveDate::parse_from_str(input, "%Y-%m-%d").ok()?;
    (input.len() == 10 && (1900..=2200).contains(&date.year())).then_some(date)
}

pub(crate) fn format_date(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// The day a yearly date (a birthday, a festival) falls on in `year`.
/// February 29 is celebrated on February 28 in common years.
pub(crate) fn yearly_in(month: u32, day: u32, year: i32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day)
        .or_else(|| NaiveDate::from_ymd_opt(year, month, day.saturating_sub(1)))
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(year, 1, 1).expect("January 1 exists"))
}

/// The next time a yearly date comes around, `today` included.
pub(crate) fn next_yearly(month: u32, day: u32, today: NaiveDate) -> NaiveDate {
    let this_year = yearly_in(month, day, today.year());
    if this_year >= today { this_year } else { yearly_in(month, day, today.year() + 1) }
}

/// Whole days from `today` until `date` (negative once it has passed).
pub(crate) fn days_until(date: NaiveDate, today: NaiveDate) -> i32 {
    (date - today).num_days().clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// "11/24(火)" and "Tue, Nov 24": a day as notification texts show it.
pub(crate) fn day_text(date: NaiveDate) -> (String, String) {
    const WEEKDAYS_JA: [&str; 7] = ["月", "火", "水", "木", "金", "土", "日"];
    let weekday = WEEKDAYS_JA[date.weekday().num_days_from_monday() as usize];
    (format!("{}/{}({weekday})", date.month(), date.day()), date.format("%a, %b %-d").to_string())
}

#[cfg(test)]
mod tests {
    use chrono::Timelike as _;

    use super::*;

    fn date(text: &str) -> NaiveDate {
        parse_date(text).unwrap()
    }

    #[test]
    fn days_begin_at_local_midnight() {
        let tokyo = chrono_tz::Asia::Tokyo;
        assert_eq!(day_start(date("2026-11-24"), tokyo).to_rfc3339(), "2026-11-23T15:00:00+00:00");
        assert_eq!(day_end(date("2026-11-24"), tokyo).to_rfc3339(), "2026-11-24T15:00:00+00:00");
        assert_eq!(local_instant(date("2026-11-24"), 18, tokyo).to_rfc3339(), "2026-11-24T09:00:00+00:00");
        assert_eq!(day_in(tokyo, day_start(date("2026-11-24"), tokyo)), date("2026-11-24"));
        assert_eq!(day_in(tokyo, day_start(date("2026-11-24"), tokyo) - Duration::seconds(1)), date("2026-11-23"));
    }

    #[test]
    fn skipped_and_repeated_hours_still_give_an_instant() {
        let new_york = chrono_tz::America::New_York;
        // 2026-03-08 02:00 does not exist there (clocks jump to 03:00).
        let skipped = local_instant(date("2026-03-08"), 2, new_york);
        assert_eq!(skipped.with_timezone(&new_york).hour(), 3);
        // 2026-11-01 01:00 happens twice; the first one (still daylight time) is taken.
        let repeated = local_instant(date("2026-11-01"), 1, new_york);
        assert_eq!(repeated.to_rfc3339(), "2026-11-01T05:00:00+00:00");
    }

    #[test]
    fn dates_are_parsed_strictly() {
        assert_eq!(parse_date("2026-02-28"), NaiveDate::from_ymd_opt(2026, 2, 28));
        for bad in
            ["", "2026-2-28", "2026-02-30", "26-02-28", "2026/02/28", "0001-01-01", "9999-01-01", "2026-02-28T00:00:00"]
        {
            assert_eq!(parse_date(bad), None, "{bad}");
        }
        assert_eq!(format_date(date("2026-02-08")), "2026-02-08");
    }

    #[test]
    fn yearly_dates_come_around_including_leap_days() {
        assert_eq!(next_yearly(12, 24, date("2026-11-22")), date("2026-12-24"));
        assert_eq!(next_yearly(11, 22, date("2026-11-22")), date("2026-11-22"));
        assert_eq!(next_yearly(11, 21, date("2026-11-22")), date("2027-11-21"));
        assert_eq!(yearly_in(2, 29, 2027), date("2027-02-28"));
        assert_eq!(yearly_in(2, 29, 2028), date("2028-02-29"));
        assert_eq!(next_yearly(2, 29, date("2027-03-01")), date("2028-02-29"));
        assert_eq!(days_until(date("2026-12-24"), date("2026-11-22")), 32);
        assert_eq!(days_until(date("2026-11-21"), date("2026-11-22")), -1);
    }

    #[test]
    fn days_are_written_with_their_weekday() {
        assert_eq!(day_text(date("2026-11-24")), ("11/24(火)".to_owned(), "Tue, Nov 24".to_owned()));
        assert_eq!(day_text(date("2027-01-03")), ("1/3(日)".to_owned(), "Sun, Jan 3".to_owned()));
    }

    #[test]
    fn unknown_zone_names_fall_back() {
        assert_eq!(zone("Europe/Paris"), chrono_tz::Europe::Paris);
        assert_eq!(zone("Mars/Olympus"), FALLBACK_ZONE);
    }
}
