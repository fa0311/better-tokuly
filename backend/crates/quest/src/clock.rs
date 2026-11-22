//! Calendar arithmetic in a player's own time zone.
//!
//! Everything in the quest domain turns over at the player's local midnight:
//! the daily XP caps and missions, the week (Monday to Sunday), the season
//! (calendar month) and each friend's part of a streak day. These helpers
//! are pure; [`crate::Quest::zone_of`] supplies the zone.
use chrono::{DateTime, Datelike as _, Duration, NaiveDate, NaiveTime, TimeZone as _, Timelike as _, Utc};
use chrono_tz::Tz;

/// The zone of accounts that never set one, and of unparsable values.
pub const DEFAULT_ZONE: Tz = chrono_tz::Asia::Tokyo;

/// Parses a stored zone name, falling back to [`DEFAULT_ZONE`].
pub fn parse_zone(name: &str) -> Tz {
    name.parse().unwrap_or(DEFAULT_ZONE)
}

/// The calendar date it is at `at` for someone living in `zone`.
pub fn local_date(at: DateTime<Utc>, zone: Tz) -> NaiveDate {
    at.with_timezone(&zone).date_naive()
}

/// The hour of the day (0-23) at `at` in `zone`.
pub fn local_hour(at: DateTime<Utc>, zone: Tz) -> u32 {
    at.with_timezone(&zone).hour()
}

/// The first instant of `date` in `zone`. Where clocks jump forward over
/// midnight the day starts at the first wall-clock time that exists.
pub fn start_of_day(date: NaiveDate, zone: Tz) -> DateTime<Utc> {
    let midnight = date.and_time(NaiveTime::MIN);
    (0..24 * 4)
        .map(|quarter| midnight + Duration::minutes(15 * quarter))
        .find_map(|wall_clock| zone.from_local_datetime(&wall_clock).earliest())
        .map(|start| start.with_timezone(&Utc))
        .unwrap_or_else(|| Utc.from_utc_datetime(&midnight))
}

/// The instant `date` is over in `zone`: the start of the next day.
pub fn end_of_day(date: NaiveDate, zone: Tz) -> DateTime<Utc> {
    start_of_day(next_day(date), zone)
}

pub fn next_day(date: NaiveDate) -> NaiveDate {
    date.succ_opt().unwrap_or(date)
}

pub fn previous_day(date: NaiveDate) -> NaiveDate {
    date.pred_opt().unwrap_or(date)
}

/// The Monday of the week `date` falls in.
pub fn week_start(date: NaiveDate) -> NaiveDate {
    date - Duration::days(date.weekday().num_days_from_monday() as i64)
}

/// The Sunday of the week `date` falls in.
pub fn week_end(date: NaiveDate) -> NaiveDate {
    week_start(date) + Duration::days(6)
}

/// The first day of the month `date` falls in.
pub fn month_start(date: NaiveDate) -> NaiveDate {
    date.with_day(1).unwrap_or(date)
}

/// The first day of the month after the one `date` falls in.
pub fn next_month_start(date: NaiveDate) -> NaiveDate {
    let (year, month) = if date.month() == 12 { (date.year() + 1, 1) } else { (date.year(), date.month() + 1) };
    NaiveDate::from_ymd_opt(year, month, 1).unwrap_or(date)
}

/// The last day of the month `date` falls in.
pub fn month_end(date: NaiveDate) -> NaiveDate {
    previous_day(next_month_start(date))
}

/// `YYYY-MM` of the month `date` falls in: the ID of a season.
pub fn month_id(date: NaiveDate) -> String {
    format!("{:04}-{:02}", date.year(), date.month())
}

/// `YYYY-MM-DD`, the way dates travel in the API and in Valkey keys.
pub fn date_text(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

pub fn parse_date(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
}

/// A calendar month is over everywhere on Earth 12 hours after it ended in
/// UTC (the last zone, UTC−12, leaves it then). Season results are final
/// from this instant on.
pub fn month_over_everywhere(month: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&next_month_start(month).and_time(NaiveTime::MIN)) + Duration::hours(12)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    #[test]
    fn the_same_instant_is_a_different_date_in_different_zones() {
        let at = utc(2026, 11, 22, 16, 30);
        assert_eq!(local_date(at, chrono_tz::Asia::Tokyo), date(2026, 11, 23));
        assert_eq!(local_date(at, chrono_tz::Europe::London), date(2026, 11, 22));
        assert_eq!(local_date(at, chrono_tz::America::Los_Angeles), date(2026, 11, 22));
        assert_eq!(local_hour(at, chrono_tz::Asia::Tokyo), 1);
    }

    #[test]
    fn days_start_and_end_at_local_midnight() {
        let day = date(2026, 11, 22);
        assert_eq!(start_of_day(day, chrono_tz::Asia::Tokyo), utc(2026, 11, 21, 15, 0));
        assert_eq!(end_of_day(day, chrono_tz::Asia::Tokyo), utc(2026, 11, 22, 15, 0));
        assert_eq!(start_of_day(day, chrono_tz::UTC), utc(2026, 11, 22, 0, 0));
    }

    #[test]
    fn a_day_that_skips_midnight_starts_when_the_clock_exists() {
        // Clocks in Beirut went from 00:00 straight to 01:00 on 2022-03-27.
        let start = start_of_day(date(2022, 3, 27), chrono_tz::Asia::Beirut);
        assert_eq!(local_date(start, chrono_tz::Asia::Beirut), date(2022, 3, 27));
        assert_eq!(local_date(start - Duration::seconds(1), chrono_tz::Asia::Beirut), date(2022, 3, 26));
    }

    #[test]
    fn weeks_run_monday_to_sunday() {
        // 2026-11-22 is a Sunday.
        assert_eq!(week_start(date(2026, 11, 22)), date(2026, 11, 16));
        assert_eq!(week_end(date(2026, 11, 22)), date(2026, 11, 22));
        assert_eq!(week_start(date(2026, 11, 23)), date(2026, 11, 23));
        assert_eq!(week_end(date(2026, 11, 16)), date(2026, 11, 22));
    }

    #[test]
    fn months_know_their_borders() {
        assert_eq!(month_start(date(2026, 11, 22)), date(2026, 11, 1));
        assert_eq!(month_end(date(2026, 11, 22)), date(2026, 11, 30));
        assert_eq!(next_month_start(date(2026, 12, 31)), date(2027, 1, 1));
        assert_eq!(month_end(date(2028, 2, 10)), date(2028, 2, 29));
        assert_eq!(month_id(date(2026, 3, 9)), "2026-03");
    }

    #[test]
    fn a_month_is_over_everywhere_twelve_hours_after_utc() {
        let over = month_over_everywhere(date(2026, 11, 1));
        assert_eq!(over, utc(2026, 12, 1, 12, 0));
        // The westernmost zone is still in November one second earlier.
        let baker = chrono_tz::Etc::GMTPlus12;
        assert_eq!(local_date(over - Duration::seconds(1), baker), date(2026, 11, 30));
        assert_eq!(local_date(over, baker), date(2026, 12, 1));
    }

    #[test]
    fn dates_round_trip_through_text_and_bad_zones_fall_back() {
        assert_eq!(parse_date(&date_text(date(2026, 1, 5))), Some(date(2026, 1, 5)));
        assert_eq!(parse_date("yesterday"), None);
        assert_eq!(parse_zone("Mars/Olympus"), DEFAULT_ZONE);
        assert_eq!(parse_zone("Europe/Paris"), chrono_tz::Europe::Paris);
    }
}
