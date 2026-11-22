//! Japanese public holidays (国民の祝日に関する法律), computed rather than
//! listed so no table has to be updated every year.
//!
//! Follows the law as it stands since 2020; the one-off moves for the Tokyo
//! Olympics (2020, 2021) are not modelled. The equinox formulas hold until 2099.
use chrono::{Datelike as _, Duration, NaiveDate, Weekday};

/// The name of the public holiday on `date`, if it is one — including
/// substitute holidays (振替休日) and days between two holidays (国民の休日).
pub(crate) fn public_holiday(date: NaiveDate) -> Option<&'static str> {
    if let Some(name) = national_holiday(date) {
        return Some(name);
    }
    // 振替休日: when a holiday falls on a Sunday, the first day after it that
    // is not a holiday itself is a day off.
    let mut day = date - Duration::days(1);
    while national_holiday(day).is_some() {
        if day.weekday() == Weekday::Sun {
            return Some("振替休日");
        }
        day -= Duration::days(1);
    }
    // 国民の休日: a day between two holidays.
    if national_holiday(date - Duration::days(1)).is_some() && national_holiday(date + Duration::days(1)).is_some() {
        return Some("国民の休日");
    }
    None
}

/// The 祝日 proper.
fn national_holiday(date: NaiveDate) -> Option<&'static str> {
    let nth_monday = |n: u32| date.weekday() == Weekday::Mon && (date.day() - 1) / 7 + 1 == n;
    match (date.month(), date.day()) {
        (1, 1) => Some("元日"),
        (1, _) if nth_monday(2) => Some("成人の日"),
        (2, 11) => Some("建国記念の日"),
        (2, 23) => Some("天皇誕生日"),
        (3, day) if day == spring_equinox_day(date.year()) => Some("春分の日"),
        (4, 29) => Some("昭和の日"),
        (5, 3) => Some("憲法記念日"),
        (5, 4) => Some("みどりの日"),
        (5, 5) => Some("こどもの日"),
        (7, _) if nth_monday(3) => Some("海の日"),
        (8, 11) => Some("山の日"),
        (9, _) if nth_monday(3) => Some("敬老の日"),
        (9, day) if day == autumn_equinox_day(date.year()) => Some("秋分の日"),
        (10, _) if nth_monday(2) => Some("スポーツの日"),
        (11, 3) => Some("文化の日"),
        (11, 23) => Some("勤労感謝の日"),
        _ => None,
    }
}

fn spring_equinox_day(year: i32) -> u32 {
    equinox_day(20.8431, year)
}

fn autumn_equinox_day(year: i32) -> u32 {
    equinox_day(23.2488, year)
}

fn equinox_day(base: f64, year: i32) -> u32 {
    let years = (year - 1980) as f64;
    ((base + 0.242194 * years).floor() - (years / 4.0).floor()) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holidays_of(year: i32) -> Vec<(u32, u32, &'static str)> {
        let mut day = NaiveDate::from_ymd_opt(year, 1, 1).unwrap();
        let mut found = Vec::new();
        while day.year() == year {
            if let Some(name) = public_holiday(day) {
                found.push((day.month(), day.day(), name));
            }
            day += Duration::days(1);
        }
        found
    }

    #[test]
    fn the_holidays_of_2026_match_the_official_calendar() {
        assert_eq!(
            holidays_of(2026),
            vec![
                (1, 1, "元日"),
                (1, 12, "成人の日"),
                (2, 11, "建国記念の日"),
                (2, 23, "天皇誕生日"),
                (3, 20, "春分の日"),
                (4, 29, "昭和の日"),
                (5, 3, "憲法記念日"),
                (5, 4, "みどりの日"),
                (5, 5, "こどもの日"),
                // 憲法記念日 fell on a Sunday; the next free day is the 6th.
                (5, 6, "振替休日"),
                (7, 20, "海の日"),
                (8, 11, "山の日"),
                (9, 21, "敬老の日"),
                // Between 敬老の日 and 秋分の日.
                (9, 22, "国民の休日"),
                (9, 23, "秋分の日"),
                (10, 12, "スポーツの日"),
                (11, 3, "文化の日"),
                (11, 23, "勤労感謝の日"),
            ]
        );
    }

    #[test]
    fn the_holidays_of_2027_match_the_official_calendar() {
        assert_eq!(
            holidays_of(2027),
            vec![
                (1, 1, "元日"),
                (1, 11, "成人の日"),
                (2, 11, "建国記念の日"),
                (2, 23, "天皇誕生日"),
                // 春分の日 is a Sunday in 2027.
                (3, 21, "春分の日"),
                (3, 22, "振替休日"),
                (4, 29, "昭和の日"),
                (5, 3, "憲法記念日"),
                (5, 4, "みどりの日"),
                (5, 5, "こどもの日"),
                (7, 19, "海の日"),
                (8, 11, "山の日"),
                (9, 20, "敬老の日"),
                (9, 23, "秋分の日"),
                (10, 11, "スポーツの日"),
                (11, 3, "文化の日"),
                (11, 23, "勤労感謝の日"),
            ]
        );
    }

    #[test]
    fn ordinary_days_are_not_holidays() {
        for (month, day) in [(4, 8), (5, 7), (9, 24), (12, 25)] {
            assert_eq!(public_holiday(NaiveDate::from_ymd_opt(2026, month, day).unwrap()), None);
        }
    }
}
