//! Milestones of a "days since" counter: which days are celebrated and when
//! they fall. Pure date arithmetic; the notifications are in `anniversaries`.
//!
//! Celebrated are 100, 200, 300 and 500 days, every 1000 days, half a year
//! (半年), and every anniversary.
use chrono::{Duration, Months, NaiveDate};
use tokuly_core::outbound::Localized;
use tokuly_proto::plans::MilestoneKind;

/// What is being celebrated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mark {
    Days(i32),
    HalfYear,
    Years(i32),
}

impl Mark {
    /// Stable name, used to remember which milestones someone was told about.
    pub(crate) fn key(&self) -> String {
        match self {
            Self::Days(days) => format!("d{days}"),
            Self::HalfYear => "half".to_owned(),
            Self::Years(years) => format!("y{years}"),
        }
    }

    pub(crate) fn label(&self) -> Localized {
        match self {
            Self::Days(days) => Localized::new(format!("{days}日記念日"), format!("{days} days")),
            Self::HalfYear => Localized::new("半年記念日", "Half a year"),
            Self::Years(1) => Localized::new("1周年", "1 year"),
            Self::Years(years) => Localized::new(format!("{years}周年"), format!("{years} years")),
        }
    }

    pub(crate) fn kind(&self) -> MilestoneKind {
        match self {
            Self::Days(_) => MilestoneKind::MILESTONE_KIND_DAYS,
            Self::HalfYear => MilestoneKind::MILESTONE_KIND_HALF_YEAR,
            Self::Years(_) => MilestoneKind::MILESTONE_KIND_YEARS,
        }
    }

    /// The number that goes with the kind (0 for half a year).
    pub(crate) fn number(&self) -> i32 {
        match self {
            Self::Days(number) | Self::Years(number) => *number,
            Self::HalfYear => 0,
        }
    }

    /// When two milestones share a day the more important one is announced:
    /// an anniversary before half a year before a day count.
    fn importance(&self) -> u8 {
        match self {
            Self::Years(_) => 0,
            Self::HalfYear => 1,
            Self::Days(_) => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Milestone {
    pub mark: Mark,
    pub date: NaiveDate,
}

/// The count shown on `today`: with `count_start_day` the first day is
/// "day 1" (1日目), otherwise it is the number of days that have passed.
pub(crate) fn day_count(start: NaiveDate, today: NaiveDate, count_start_day: bool) -> i64 {
    (today - start).num_days() + i64::from(count_start_day)
}

/// The day counts that are celebrated, ascending and without end.
fn celebrated_days() -> impl Iterator<Item = i32> {
    [100, 200, 300, 500].into_iter().chain((1..).map(|thousands| thousands * 1000))
}

/// Every milestone falling on a day in `from ..= to`, in order of date (the
/// more important first when two share a day).
pub(crate) fn between(start: NaiveDate, count_start_day: bool, from: NaiveDate, to: NaiveDate) -> Vec<Milestone> {
    let mut found = Vec::new();
    if to < from || to < start {
        return found;
    }
    // "Day N" is N - 1 days after the start when the start counts as day 1.
    let offset = i64::from(count_start_day);
    for days in celebrated_days() {
        let Some(date) = start.checked_add_signed(Duration::days(days as i64 - offset)) else { break };
        if date > to {
            break;
        }
        found.push(Milestone { mark: Mark::Days(days), date });
    }
    if let Some(date) = start.checked_add_months(Months::new(6)) {
        found.push(Milestone { mark: Mark::HalfYear, date });
    }
    for years in 1..=300u32 {
        let Some(date) = start.checked_add_months(Months::new(12 * years)) else { break };
        if date > to {
            break;
        }
        found.push(Milestone { mark: Mark::Years(years as i32), date });
    }
    found.retain(|milestone| milestone.date >= from && milestone.date <= to);
    found.sort_by_key(|milestone| (milestone.date, milestone.mark.importance()));
    found
}

/// The milestone to announce on `date`, if it is one.
pub(crate) fn on(start: NaiveDate, count_start_day: bool, date: NaiveDate) -> Option<Milestone> {
    between(start, count_start_day, date, date).into_iter().next()
}

/// The next `count` milestones from `from` on (that day included), one per day.
pub(crate) fn upcoming(start: NaiveDate, count_start_day: bool, from: NaiveDate, count: usize) -> Vec<Milestone> {
    // There is an anniversary every year, so a window of `count` years plus
    // one always holds enough milestones.
    let Some(horizon) = from.checked_add_months(Months::new(12 * (count as u32 + 1))) else { return Vec::new() };
    let mut found = between(start, count_start_day, from, horizon);
    found.dedup_by_key(|milestone| milestone.date);
    found.truncate(count);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    fn dated(mark: Mark, on: &str) -> Milestone {
        Milestone { mark, date: date(on) }
    }

    #[test]
    fn the_first_day_counts_as_day_one_when_asked_to() {
        let start = date("2026-01-01");
        assert_eq!(day_count(start, start, true), 1);
        assert_eq!(day_count(start, start, false), 0);
        assert_eq!(day_count(start, date("2026-04-10"), true), 100);
        assert_eq!(day_count(start, date("2026-04-11"), false), 100);
    }

    #[test]
    fn a_first_year_holds_three_day_counts_half_a_year_and_the_anniversary() {
        let start = date("2026-01-01");
        let year = between(start, true, start, date("2027-01-01"));
        assert_eq!(
            year,
            vec![
                dated(Mark::Days(100), "2026-04-10"),
                dated(Mark::HalfYear, "2026-07-01"),
                dated(Mark::Days(200), "2026-07-19"),
                dated(Mark::Days(300), "2026-10-27"),
                dated(Mark::Years(1), "2027-01-01"),
            ]
        );
        // Counting elapsed days moves the day counts one day later, nothing else.
        let elapsed = between(start, false, start, date("2027-01-01"));
        assert_eq!(elapsed[0], dated(Mark::Days(100), "2026-04-11"));
        assert_eq!(elapsed[1], dated(Mark::HalfYear, "2026-07-01"));
    }

    #[test]
    fn later_years_bring_anniversaries_and_round_thousands() {
        let start = date("2020-02-29");
        let later = between(start, true, date("2022-01-01"), date("2025-12-31"));
        assert_eq!(
            later,
            vec![
                // The leap day's anniversary is kept on February 28 in common years.
                dated(Mark::Years(2), "2022-02-28"),
                dated(Mark::Days(1000), "2022-11-24"),
                dated(Mark::Years(3), "2023-02-28"),
                dated(Mark::Years(4), "2024-02-29"),
                dated(Mark::Years(5), "2025-02-28"),
                dated(Mark::Days(2000), "2025-08-20"),
            ]
        );
    }

    #[test]
    fn only_the_day_itself_is_a_milestone() {
        let start = date("2026-01-01");
        assert_eq!(on(start, true, date("2026-04-10")), Some(dated(Mark::Days(100), "2026-04-10")));
        assert_eq!(on(start, true, date("2026-04-11")), None);
        assert_eq!(on(start, true, date("2025-12-31")), None);
        assert_eq!(on(start, true, date("2028-01-01")).map(|m| m.mark), Some(Mark::Years(2)));
    }

    #[test]
    fn an_anniversary_wins_over_a_day_count_on_the_same_day() {
        // Real counters hardly ever have two milestones on one day, so the
        // rule that picks between them is checked on its own.
        let mut same_day = vec![
            dated(Mark::Days(1000), "2026-01-01"),
            dated(Mark::Years(3), "2026-01-01"),
            dated(Mark::HalfYear, "2026-01-01"),
        ];
        same_day.sort_by_key(|milestone| (milestone.date, milestone.mark.importance()));
        assert_eq!(
            same_day.iter().map(|m| m.mark).collect::<Vec<_>>(),
            vec![Mark::Years(3), Mark::HalfYear, Mark::Days(1000)]
        );
    }

    #[test]
    fn upcoming_lists_the_next_few_from_today() {
        let start = date("2026-01-01");
        let next = upcoming(start, true, date("2026-07-01"), 3);
        assert_eq!(
            next,
            vec![
                dated(Mark::HalfYear, "2026-07-01"),
                dated(Mark::Days(200), "2026-07-19"),
                dated(Mark::Days(300), "2026-10-27")
            ]
        );
        // Before the start nothing is hidden: the first milestones are simply further away.
        assert_eq!(upcoming(start, true, date("2025-12-01"), 1), vec![dated(Mark::Days(100), "2026-04-10")]);
        assert_eq!(upcoming(start, true, date("2031-06-01"), 2).len(), 2);
    }

    #[test]
    fn marks_have_stable_keys_and_labels_in_both_languages() {
        assert_eq!(Mark::Days(100).key(), "d100");
        assert_eq!(Mark::HalfYear.key(), "half");
        assert_eq!(Mark::Years(2).key(), "y2");
        assert_eq!(Mark::Days(100).label().ja, "100日記念日");
        assert_eq!(Mark::HalfYear.label().ja, "半年記念日");
        assert_eq!(Mark::Years(1).label().en, "1 year");
        assert_eq!(Mark::Years(3).label().ja, "3周年");
        assert_eq!((Mark::Years(3).kind(), Mark::Years(3).number()), (MilestoneKind::MILESTONE_KIND_YEARS, 3));
        assert_eq!(Mark::HalfYear.number(), 0);
    }
}
