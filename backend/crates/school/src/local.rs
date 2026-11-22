//! Calendar dates and local time: everything "per day" in this domain is
//! judged in the user's time zone (`users.time_zone`, Asia/Tokyo by default).
use chrono::{DateTime, Datelike as _, Duration, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone as _};
use chrono_tz::Tz;
use tokuly_core::prelude::*;

pub(crate) const DEFAULT_TIME_ZONE: Tz = chrono_tz::Asia::Tokyo;

/// Parses a `YYYY-MM-DD` date from a request.
pub(crate) fn parse_date(value: &str) -> AppResult<NaiveDate> {
    NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d")
        .ok()
        .filter(|date| (2000..=2100).contains(&date.year()))
        .ok_or_else(|| {
            AppError::invalid("INVALID_DATE", format!("not a date between 2000 and 2100 (YYYY-MM-DD): {value:?}"))
        })
}

/// Parses an optional date; an empty string means "not given".
pub(crate) fn parse_opt_date(value: &str) -> AppResult<Option<NaiveDate>> {
    if value.trim().is_empty() { Ok(None) } else { parse_date(value).map(Some) }
}

pub(crate) fn fmt_date(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

pub(crate) fn fmt_opt_date(date: Option<NaiveDate>) -> String {
    date.map(fmt_date).unwrap_or_default()
}

/// The time zone a user's days are counted in.
pub(crate) async fn time_zone(core: &Core, user_id: Uuid) -> AppResult<Tz> {
    time_zone_on(&mut *core.db.acquire().await?, user_id).await
}

/// [`time_zone`] on a connection the caller holds (inside a transaction).
pub(crate) async fn time_zone_on(conn: &mut sqlx::PgConnection, user_id: Uuid) -> AppResult<Tz> {
    let name: Option<String> =
        sqlx::query_scalar("SELECT time_zone FROM users WHERE id = $1").bind(user_id).fetch_optional(conn).await?;
    Ok(parse_time_zone(name.as_deref()))
}

pub(crate) fn parse_time_zone(name: Option<&str>) -> Tz {
    name.and_then(|name| name.parse().ok()).unwrap_or(DEFAULT_TIME_ZONE)
}

/// The calendar date it is at `at` in `tz`.
pub(crate) fn local_date(at: Time, tz: Tz) -> NaiveDate {
    at.with_timezone(&tz).date_naive()
}

/// The instant of a local wall-clock time. A time skipped by a clock change
/// resolves to the first valid instant after it; an ambiguous one to the earlier.
pub(crate) fn local_instant(local: NaiveDateTime, tz: Tz) -> Time {
    let mut local = local;
    // Clock changes skip at most a couple of hours; step in quarter hours.
    for _ in 0..16 {
        match tz.from_local_datetime(&local) {
            LocalResult::Single(at) => return at.to_utc(),
            LocalResult::Ambiguous(earlier, _) => return earlier.to_utc(),
            LocalResult::None => local += Duration::minutes(15),
        }
    }
    DateTime::from_naive_utc_and_offset(local, chrono::Utc)
}

/// Local midnight at the start of `date`.
pub(crate) fn day_start(date: NaiveDate, tz: Tz) -> Time {
    local_instant(date.and_time(NaiveTime::MIN), tz)
}

/// `minute` minutes after local midnight on `date` (1440 = the next midnight).
pub(crate) fn at_minute(date: NaiveDate, minute: i32, tz: Tz) -> Time {
    local_instant(date.and_time(NaiveTime::MIN) + Duration::minutes(minute as i64), tz)
}

/// 1 = Monday ... 7 = Sunday, as in `tokuly.school.v1.Weekday`.
pub(crate) fn weekday_of(date: NaiveDate) -> i32 {
    date.weekday().number_from_monday() as i32
}

/// The Monday of the week `date` is in.
pub(crate) fn week_start(date: NaiveDate) -> NaiveDate {
    date - Duration::days(date.weekday().num_days_from_monday() as i64)
}

pub(crate) fn month_start(date: NaiveDate) -> NaiveDate {
    date.with_day(1).expect("every month has a first day")
}

/// The first day of the month after the one `date` is in.
pub(crate) fn next_month_start(date: NaiveDate) -> NaiveDate {
    let (year, month) = if date.month() == 12 { (date.year() + 1, 1) } else { (date.year(), date.month() + 1) };
    NaiveDate::from_ymd_opt(year, month, 1).expect("the first of a month exists")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokyo(y: i32, m: u32, d: u32, h: u32, min: u32) -> Time {
        chrono_tz::Asia::Tokyo.with_ymd_and_hms(y, m, d, h, min, 0).unwrap().to_utc()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn dates_parse_strictly() {
        assert_eq!(parse_date("2026-04-08").unwrap(), date(2026, 4, 8));
        for bad in ["", "2026-13-01", "2026/04/08", "20260408", "1999-12-31", "2101-01-01", "2026-02-30"] {
            assert_eq!(parse_date(bad).unwrap_err().reason, "INVALID_DATE", "{bad}");
        }
        assert_eq!(parse_opt_date("  ").unwrap(), None);
    }

    #[test]
    fn local_dates_follow_the_time_zone() {
        // 23:30 UTC on the 7th is already the 8th in Tokyo and still the 7th in New York.
        let at = chrono::Utc.with_ymd_and_hms(2026, 4, 7, 23, 30, 0).unwrap();
        assert_eq!(local_date(at, chrono_tz::Asia::Tokyo), date(2026, 4, 8));
        assert_eq!(local_date(at, chrono_tz::America::New_York), date(2026, 4, 7));
        assert_eq!(
            day_start(date(2026, 4, 8), chrono_tz::Asia::Tokyo),
            chrono::Utc.with_ymd_and_hms(2026, 4, 7, 15, 0, 0).unwrap()
        );
        assert_eq!(at_minute(date(2026, 4, 8), 530, chrono_tz::Asia::Tokyo), tokyo(2026, 4, 8, 8, 50));
        assert_eq!(at_minute(date(2026, 4, 8), 1440, chrono_tz::Asia::Tokyo), tokyo(2026, 4, 9, 0, 0));
    }

    #[test]
    fn skipped_and_repeated_local_times_resolve() {
        let new_york = chrono_tz::America::New_York;
        // 02:30 does not exist on 2026-03-08 in New York: the clock jumps from 02:00 to 03:00.
        let skipped = local_instant(date(2026, 3, 8).and_hms_opt(2, 30, 0).unwrap(), new_york);
        assert_eq!(skipped, chrono::Utc.with_ymd_and_hms(2026, 3, 8, 7, 0, 0).unwrap());
        // 01:30 happens twice on 2026-11-01: the earlier one (daylight time) is used.
        let repeated = local_instant(date(2026, 11, 1).and_hms_opt(1, 30, 0).unwrap(), new_york);
        assert_eq!(repeated, chrono::Utc.with_ymd_and_hms(2026, 11, 1, 5, 30, 0).unwrap());
    }

    #[test]
    fn weeks_start_on_monday() {
        assert_eq!(weekday_of(date(2026, 4, 6)), 1); // a Monday
        assert_eq!(weekday_of(date(2026, 4, 12)), 7);
        assert_eq!(week_start(date(2026, 4, 12)), date(2026, 4, 6));
        assert_eq!(week_start(date(2026, 4, 6)), date(2026, 4, 6));
        assert_eq!(month_start(date(2026, 12, 31)), date(2026, 12, 1));
        assert_eq!(next_month_start(date(2026, 12, 31)), date(2027, 1, 1));
        assert_eq!(next_month_start(date(2026, 1, 31)), date(2026, 2, 1));
    }

    #[test]
    fn a_day_with_a_clock_change_is_23_hours_long() {
        let new_york = chrono_tz::America::New_York;
        let start = day_start(date(2026, 3, 8), new_york);
        let end = day_start(date(2026, 3, 9), new_york);
        assert_eq!((end - start).num_hours(), 23);
    }
}
