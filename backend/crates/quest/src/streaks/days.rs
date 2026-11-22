//! The streak day logic: a pure state machine over calendar dates.
//!
//! # What a streak day is
//!
//! A streak day is a calendar date. A friend "does their part" for a date by
//! doing something towards the other **while it is that date on their own
//! clock**. The date is *complete* once both did their part, and the streak
//! is the number of consecutive complete dates.
//!
//! Two friends in different time zones are on different dates for part of
//! every day, so the two parts of one date may arrive hours apart and in
//! either order, and one side can already be working on the next date. What
//! never happens is that a time zone shortens somebody's day: each friend
//! has their whole local day, until their own midnight, to do their part.
//!
//! # How dates are settled
//!
//! Dates are settled strictly in order. The date after the last settled one
//! is
//!
//! - **complete** when both sides did their part: the streak grows by one;
//! - **missed** when a side that did not do its part has run out of day
//!   (its local midnight has passed): a running streak ends there;
//! - otherwise still **open**, and nothing later is looked at.
//!
//! So a running streak ends at the earliest local midnight among the sides
//! whose part for the open date is missing — [`Days::deadline`]. A streak
//! that ended is remembered for a while ([`Days::lost_count`]) and can be
//! restored ([`Days::restore`]), which forgives every date up to yesterday.
//!
//! `settled` only moves forward, and parts reported for a settled date are
//! ignored. That makes the machine indifferent to redelivered events: once
//! a part is recorded or its date settled, reporting it again changes
//! nothing.
//!
//! # Late events
//!
//! Events reach the quest domain a moment after they happen. A running
//! streak is therefore only declared missed [`Limits::grace`] after the
//! midnight in question, so a message sent at 23:59:58 still counts when it
//! arrives at 00:00:03. The deadline shown to players has no grace.
use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Tz;

use crate::clock::{end_of_day, local_date, next_day, previous_day};

/// Streak lengths that are celebrated (and come with a badge).
pub const MILESTONES: &[i32] = &[3, 7, 30, 100, 365, 500, 1000];

/// The first milestone above `days`; 0 past the last one.
pub fn next_milestone(days: i32) -> i32 {
    MILESTONES.iter().copied().find(|milestone| *milestone > days).unwrap_or(0)
}

pub fn is_milestone(days: i32) -> bool {
    MILESTONES.contains(&days)
}

/// One of the two friends: the one with the smaller or the larger user ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Lo,
    Hi,
}

/// The time zones of the two friends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Zones {
    pub lo: Tz,
    pub hi: Tz,
}

impl Zones {
    pub fn of(&self, side: Side) -> Tz {
        match side {
            Side::Lo => self.lo,
            Side::Hi => self.hi,
        }
    }
}

/// The time allowances of the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// How long after a midnight a running streak is declared missed.
    pub grace: Duration,
    /// How long a streak that ended can be restored.
    pub restore_window: Duration,
}

/// The streak between two friends.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Days {
    /// Consecutive complete dates; 0 = no streak is running.
    pub count: i32,
    /// The longest streak the two ever had.
    pub best: i32,
    /// Every date up to this one is settled (complete or missed). While a
    /// streak runs it is the streak's latest complete date.
    pub settled: Option<NaiveDate>,
    /// First date of the running streak.
    pub started_on: Option<NaiveDate>,
    /// Dates after `settled` for which each side did its part, ascending.
    pub lo_days: Vec<NaiveDate>,
    pub hi_days: Vec<NaiveDate>,
    /// Length of the streak that ended last (0 = none to restore) and when
    /// it ended: the midnight that passed.
    pub lost_count: i32,
    pub lost_at: Option<DateTime<Utc>>,
}

/// Something that happened while settling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// A date became complete; the streak is now `days` long.
    Completed { date: NaiveDate, days: i32 },
    /// The running streak of `days` ended at `at`.
    Lost { days: i32, at: DateTime<Utc> },
}

/// Whose part of the open date is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Missing {
    pub lo: bool,
    pub hi: bool,
}

impl Missing {
    pub fn of(&self, side: Side) -> bool {
        match side {
            Side::Lo => self.lo,
            Side::Hi => self.hi,
        }
    }
}

impl Days {
    fn days_of(&mut self, side: Side) -> &mut Vec<NaiveDate> {
        match side {
            Side::Lo => &mut self.lo_days,
            Side::Hi => &mut self.hi_days,
        }
    }

    /// The date being worked on: the one after the last complete date while
    /// a streak runs, else the earliest date anybody did a part for.
    fn open_date(&self) -> Option<NaiveDate> {
        if self.count > 0 {
            return self.settled.map(next_day);
        }
        match (self.lo_days.first(), self.hi_days.first()) {
            (Some(lo), Some(hi)) => Some(*lo.min(hi)),
            (Some(day), None) | (None, Some(day)) => Some(*day),
            (None, None) => None,
        }
    }

    /// Whose part of `date` is missing.
    fn missing(&self, date: NaiveDate) -> Missing {
        Missing { lo: !self.lo_days.contains(&date), hi: !self.hi_days.contains(&date) }
    }

    /// Whose part the running streak is waiting for. Nobody's when no streak runs.
    pub fn waiting_on(&self) -> Missing {
        match self.open_date() {
            Some(date) if self.count > 0 => self.missing(date),
            _ => Missing::default(),
        }
    }

    /// When the running streak ends unless the missing part arrives: the
    /// earliest local midnight among the sides it is waiting for. `None`
    /// when no streak runs.
    pub fn deadline(&self, zones: Zones) -> Option<DateTime<Utc>> {
        if self.count == 0 {
            return None;
        }
        let date = self.open_date()?;
        self.out_of_day_at(date, self.missing(date), zones)
    }

    /// The earliest moment a side whose part is missing runs out of `date`.
    fn out_of_day_at(&self, date: NaiveDate, missing: Missing, zones: Zones) -> Option<DateTime<Utc>> {
        let lo = missing.lo.then(|| end_of_day(date, zones.lo));
        let hi = missing.hi.then(|| end_of_day(date, zones.hi));
        match (lo, hi) {
            (Some(lo), Some(hi)) => Some(lo.min(hi)),
            (one, other) => one.or(other),
        }
    }

    /// When `side` itself runs out of time for its missing part; `None`
    /// when the streak is not waiting for that side.
    pub fn deadline_of(&self, side: Side, zones: Zones) -> Option<DateTime<Utc>> {
        if self.count == 0 {
            return None;
        }
        let date = self.open_date()?;
        self.missing(date).of(side).then(|| end_of_day(date, zones.of(side)))
    }

    /// The date `side` was (or would be) warned about: the open date of the
    /// running streak.
    pub fn open_streak_date(&self) -> Option<NaiveDate> {
        if self.count > 0 { self.open_date() } else { None }
    }

    /// Settles every date that can be settled at `now`.
    pub fn settle(&mut self, now: DateTime<Utc>, zones: Zones, limits: Limits) -> Vec<Step> {
        let mut steps = Vec::new();
        while let Some(date) = self.open_date() {
            let missing = self.missing(date);
            if !missing.lo && !missing.hi {
                self.forget_up_to(date);
                if self.count == 0 {
                    self.started_on = Some(date);
                }
                self.count += 1;
                self.best = self.best.max(self.count);
                self.settled = Some(date);
                steps.push(Step::Completed { date, days: self.count });
                continue;
            }
            // Only a running streak is protected against late events.
            let grace = if self.count > 0 { limits.grace } else { Duration::zero() };
            let Some(out_of_day) = self.out_of_day_at(date, missing, zones).filter(|at| *at + grace <= now) else {
                break;
            };
            if self.count > 0 {
                // A longer streak that can still be restored is not replaced
                // by a shorter one ending right after it.
                let restorable = self.lost_count > 0 && self.lost_at.is_some_and(|at| at + limits.restore_window > now);
                if !restorable || self.count >= self.lost_count {
                    self.lost_count = self.count;
                    self.lost_at = Some(out_of_day);
                }
                steps.push(Step::Lost { days: self.count, at: out_of_day });
                self.count = 0;
                self.started_on = None;
            }
            self.forget_up_to(date);
            self.settled = Some(date);
        }
        steps
    }

    /// Drops the recorded parts for `date` and everything before it.
    fn forget_up_to(&mut self, date: NaiveDate) {
        self.lo_days.retain(|day| *day > date);
        self.hi_days.retain(|day| *day > date);
    }

    /// Records that `side` did its part at `at` (on the date it was for
    /// them), and settles. Reporting the same part again changes nothing.
    pub fn act(&mut self, side: Side, at: DateTime<Utc>, zones: Zones, limits: Limits) -> Vec<Step> {
        let mut steps = self.settle(at, zones, limits);
        let date = local_date(at, zones.of(side));
        if self.settled.is_some_and(|settled| date <= settled) {
            return steps;
        }
        let days = self.days_of(side);
        if let Err(position) = days.binary_search(&date) {
            days.insert(position, date);
            steps.extend(self.settle(at, zones, limits));
        }
        steps
    }

    /// Whether a streak that ended can still be restored at `now`.
    pub fn restorable(&self, now: DateTime<Utc>, limits: Limits) -> bool {
        self.lost_count >= MIN_RESTORABLE_DAYS && self.lost_at.is_some_and(|at| now < at + limits.restore_window)
    }

    /// Brings the lost streak back: every date up to yesterday (of whichever
    /// friend is further ahead) is forgiven, days completed since the loss
    /// are added on top, and both have today to continue. Returns the
    /// restored length.
    pub fn restore(&mut self, now: DateTime<Utc>, zones: Zones, limits: Limits) -> i32 {
        let today = local_date(now, zones.lo).max(local_date(now, zones.hi));
        let floor = previous_day(today);
        let settled = self.settled.map_or(floor, |settled| settled.max(floor));
        self.count += self.lost_count;
        self.best = self.best.max(self.count);
        self.settled = Some(settled);
        self.started_on = Some(settled - Duration::days(self.count as i64 - 1));
        self.forget_up_to(settled);
        self.lost_count = 0;
        self.lost_at = None;
        self.settle(now, zones, limits);
        self.count
    }
}

/// Shortest ended streak worth restoring: a single day is not a streak.
pub const MIN_RESTORABLE_DAYS: i32 = 2;

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use chrono::TimeZone as _;

    use super::*;

    const TOKYO: Tz = chrono_tz::Asia::Tokyo;
    const LONDON: Tz = chrono_tz::Europe::London;

    fn limits() -> Limits {
        Limits { grace: Duration::minutes(10), restore_window: Duration::hours(24) }
    }

    fn same_zone() -> Zones {
        Zones { lo: TOKYO, hi: TOKYO }
    }

    fn date(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
    }

    /// `d` November 2026, `h`:`m` in Tokyo.
    fn tokyo(d: u32, h: u32, m: u32) -> DateTime<Utc> {
        TOKYO.with_ymd_and_hms(2026, 11, d, h, m, 0).unwrap().with_timezone(&Utc)
    }

    fn london(d: u32, h: u32, m: u32) -> DateTime<Utc> {
        LONDON.with_ymd_and_hms(2026, 11, d, h, m, 0).unwrap().with_timezone(&Utc)
    }

    /// Both act once a day at the given Tokyo hours, for the days in `range`.
    fn both_daily(days: &mut Days, range: std::ops::RangeInclusive<u32>) {
        for d in range {
            days.act(Side::Lo, tokyo(d, 9, 0), same_zone(), limits());
            days.act(Side::Hi, tokyo(d, 21, 0), same_zone(), limits());
        }
    }

    #[test]
    fn a_day_counts_once_both_did_their_part() {
        let mut days = Days::default();
        assert!(days.act(Side::Lo, tokyo(1, 9, 0), same_zone(), limits()).is_empty());
        assert_eq!(days.count, 0);
        // More from the same side does not help.
        assert!(days.act(Side::Lo, tokyo(1, 10, 0), same_zone(), limits()).is_empty());
        let steps = days.act(Side::Hi, tokyo(1, 22, 0), same_zone(), limits());
        assert_eq!(steps, vec![Step::Completed { date: date(1), days: 1 }]);
        assert_eq!((days.count, days.started_on, days.settled), (1, Some(date(1)), Some(date(1))));
        assert!(days.lo_days.is_empty() && days.hi_days.is_empty());
    }

    #[test]
    fn consecutive_days_grow_the_streak_and_a_missed_day_ends_it() {
        let mut days = Days::default();
        both_daily(&mut days, 1..=5);
        assert_eq!((days.count, days.best), (5, 5));
        // Day 6: only one of them shows up.
        days.act(Side::Lo, tokyo(6, 9, 0), same_zone(), limits());
        assert_eq!(days.waiting_on(), Missing { lo: false, hi: true });
        assert_eq!(days.deadline(same_zone()), Some(tokyo(7, 0, 0)));
        // Day 7, well past midnight: the streak is gone when anything is settled.
        let steps = days.settle(tokyo(7, 8, 0), same_zone(), limits());
        assert_eq!(steps, vec![Step::Lost { days: 5, at: tokyo(7, 0, 0) }]);
        assert_eq!((days.count, days.best, days.lost_count, days.lost_at), (0, 5, 5, Some(tokyo(7, 0, 0))));
        assert_eq!(days.deadline(same_zone()), None);
        // They start over.
        both_daily(&mut days, 7..=8);
        assert_eq!((days.count, days.best, days.started_on), (2, 5, Some(date(7))));
    }

    #[test]
    fn a_day_off_by_both_ends_the_streak_too() {
        let mut days = Days::default();
        both_daily(&mut days, 1..=3);
        let steps = days.act(Side::Lo, tokyo(5, 12, 0), same_zone(), limits());
        assert_eq!(steps, vec![Step::Lost { days: 3, at: tokyo(5, 0, 0) }]);
        assert_eq!(days.count, 0);
        assert_eq!(days.lo_days, vec![date(5)]);
    }

    #[test]
    fn a_running_streak_survives_until_the_grace_after_midnight() {
        let mut days = Days::default();
        both_daily(&mut days, 1..=3);
        days.act(Side::Lo, tokyo(4, 9, 0), same_zone(), limits());
        // Five minutes past midnight nothing is declared yet...
        assert!(days.settle(tokyo(5, 0, 5), same_zone(), limits()).is_empty());
        assert_eq!(days.count, 3);
        // ...so the other side's message from 23:59, arriving late, still counts.
        let steps = days.act(Side::Hi, tokyo(4, 23, 59), same_zone(), limits());
        assert_eq!(steps, vec![Step::Completed { date: date(4), days: 4 }]);
        // Without it, ten minutes past midnight the streak would have ended.
        let mut other = Days::default();
        both_daily(&mut other, 1..=3);
        assert_eq!(
            other.settle(tokyo(5, 0, 10), same_zone(), limits()),
            vec![Step::Lost { days: 3, at: tokyo(5, 0, 0) }]
        );
    }

    #[test]
    fn events_arriving_out_of_order_around_midnight_do_not_break_a_streak() {
        let mut days = Days::default();
        both_daily(&mut days, 1..=3);
        days.act(Side::Hi, tokyo(4, 20, 0), same_zone(), limits());
        // Lo's message of 00:00:30 on the 5th is handled before their 23:59 one.
        assert!(days.act(Side::Lo, tokyo(5, 0, 0) + Duration::seconds(30), same_zone(), limits()).is_empty());
        let steps = days.act(Side::Lo, tokyo(4, 23, 59), same_zone(), limits());
        assert_eq!(steps, vec![Step::Completed { date: date(4), days: 4 }]);
        assert_eq!(days.lo_days, vec![date(5)]);
    }

    #[test]
    fn each_friend_has_their_own_local_day() {
        // Lo lives in Tokyo, Hi in London (nine hours behind in November).
        let zones = Zones { lo: TOKYO, hi: LONDON };
        let mut days = Days::default();
        // The 10th: Tokyo acts in its morning, London in its evening — which
        // is already the 11th in Tokyo. Still the same streak day.
        days.act(Side::Lo, tokyo(10, 8, 0), zones, limits());
        let steps = days.act(Side::Hi, london(10, 22, 0), zones, limits());
        assert_eq!(steps, vec![Step::Completed { date: date(10), days: 1 }]);
        // Tokyo is ahead: it does its part for the 11th before London has
        // even started that date.
        assert_eq!(local_date(london(10, 23, 0), TOKYO), date(11));
        days.act(Side::Lo, london(10, 23, 0), zones, limits());
        assert_eq!(days.lo_days, vec![date(11)]);
        assert_eq!(days.waiting_on(), Missing { lo: false, hi: true });
        // London has until its own midnight after the 11th.
        assert_eq!(days.deadline(zones), Some(london(12, 0, 0)));
        assert_eq!(days.deadline_of(Side::Hi, zones), Some(london(12, 0, 0)));
        assert_eq!(days.deadline_of(Side::Lo, zones), None);
        let steps = days.act(Side::Hi, london(11, 23, 30), zones, limits());
        assert_eq!(steps, vec![Step::Completed { date: date(11), days: 2 }]);
    }

    #[test]
    fn the_streak_ends_at_the_first_midnight_of_a_side_that_is_missing() {
        let zones = Zones { lo: TOKYO, hi: LONDON };
        let mut days = Days::default();
        for d in 10..=12 {
            days.act(Side::Lo, tokyo(d, 12, 0), zones, limits());
            days.act(Side::Hi, london(d, 12, 0), zones, limits());
        }
        assert_eq!(days.count, 3);
        // Nobody has done the 13th yet: Tokyo's midnight comes first.
        assert_eq!(days.waiting_on(), Missing { lo: true, hi: true });
        assert_eq!(days.deadline(zones), Some(tokyo(14, 0, 0)));
        // Once Tokyo did its part, only London's midnight matters.
        days.act(Side::Lo, tokyo(13, 23, 0), zones, limits());
        assert_eq!(days.deadline(zones), Some(london(14, 0, 0)));
        // London lets it pass.
        let steps = days.settle(london(14, 0, 11), zones, limits());
        assert_eq!(steps, vec![Step::Lost { days: 3, at: london(14, 0, 0) }]);
    }

    #[test]
    fn restoring_forgives_the_gap_and_keeps_days_done_since() {
        let mut days = Days::default();
        both_daily(&mut days, 1..=6);
        // The 7th is missed; on the 8th both are back before anyone restores.
        days.act(Side::Lo, tokyo(8, 9, 0), same_zone(), limits());
        days.act(Side::Hi, tokyo(8, 10, 0), same_zone(), limits());
        assert_eq!((days.count, days.lost_count, days.lost_at), (1, 6, Some(tokyo(8, 0, 0))));
        assert!(days.restorable(tokyo(8, 23, 0), limits()));
        assert!(!days.restorable(tokyo(9, 0, 0), limits()));
        let restored = days.restore(tokyo(8, 12, 0), same_zone(), limits());
        assert_eq!(restored, 7);
        assert_eq!((days.count, days.best, days.lost_count, days.lost_at), (7, 7, 0, None));
        assert_eq!(days.settled, Some(date(8)));
        // And it simply goes on.
        both_daily(&mut days, 9..=9);
        assert_eq!(days.count, 8);
    }

    #[test]
    fn a_restored_streak_gives_both_until_tonight() {
        let mut days = Days::default();
        both_daily(&mut days, 1..=4);
        days.settle(tokyo(6, 1, 0), same_zone(), limits());
        assert_eq!((days.count, days.lost_count), (0, 4));
        let restored = days.restore(tokyo(6, 20, 0), same_zone(), limits());
        assert_eq!(restored, 4);
        assert_eq!(days.settled, Some(date(5)));
        assert_eq!(days.waiting_on(), Missing { lo: true, hi: true });
        assert_eq!(days.deadline(same_zone()), Some(tokyo(7, 0, 0)));
    }

    #[test]
    fn a_short_streak_ending_does_not_replace_a_longer_restorable_one() {
        let mut days = Days::default();
        both_daily(&mut days, 1..=10);
        // Lost on the 11th; a one-day streak on the 12th is lost on the 13th.
        both_daily(&mut days, 12..=12);
        assert_eq!((days.count, days.lost_count), (1, 10));
        days.settle(tokyo(14, 0, 30), same_zone(), limits());
        // The 10-day streak ended at the start of the 12th: at 00:30 on the
        // 14th its window is over, so the newer loss takes its place.
        assert_eq!((days.count, days.lost_count, days.lost_at), (0, 1, Some(tokyo(14, 0, 0))));

        let mut days = Days::default();
        both_daily(&mut days, 1..=10);
        days.act(Side::Lo, tokyo(11, 23, 0), same_zone(), limits());
        days.act(Side::Hi, tokyo(12, 0, 30), same_zone(), limits());
        // Lost at midnight after the 11th; within the window a later short
        // loss would not overwrite it.
        assert_eq!(days.lost_count, 10);
        assert!(
            !Days { lost_count: 1, lost_at: Some(tokyo(12, 0, 0)), ..Default::default() }
                .restorable(tokyo(12, 1, 0), limits())
        );
    }

    #[test]
    fn milestones_are_known() {
        assert_eq!(next_milestone(0), 3);
        assert_eq!(next_milestone(3), 7);
        assert_eq!(next_milestone(364), 365);
        assert_eq!(next_milestone(1000), 0);
        assert!(is_milestone(30) && !is_milestone(31));
    }

    // ── property-style tests: many random histories ──

    /// SplitMix64: small, seedable, good enough to generate test histories.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }

        fn chance(&mut self, percent: u64) -> bool {
            self.below(100) < percent
        }
    }

    /// Zones from one end of the map to the other, with odd offsets and
    /// daylight saving in between.
    const ZONES: &[Tz] = &[
        chrono_tz::Pacific::Kiritimati,
        chrono_tz::Pacific::Chatham,
        chrono_tz::Asia::Tokyo,
        chrono_tz::Asia::Kolkata,
        chrono_tz::Europe::London,
        chrono_tz::America::New_York,
        chrono_tz::America::Los_Angeles,
        chrono_tz::Etc::GMTPlus12,
        chrono_tz::UTC,
    ];

    /// A random history: who acted when, in chronological order, over about
    /// three weeks starting just before a daylight saving change.
    fn history(rng: &mut Rng) -> (Zones, Vec<(Side, DateTime<Utc>)>) {
        let zones = Zones {
            lo: ZONES[rng.below(ZONES.len() as u64) as usize],
            hi: ZONES[rng.below(ZONES.len() as u64) as usize],
        };
        let start = Utc.with_ymd_and_hms(2026, 10, 20, 0, 0, 0).unwrap();
        // How eager each side is: some pairs keep long streaks, some break a lot.
        let eagerness = [40 + rng.below(60), 40 + rng.below(60)];
        let mut events = Vec::new();
        let mut at = start;
        while at < start + Duration::days(21) {
            at += Duration::minutes(20 + rng.below(9 * 60) as i64);
            let side = if rng.chance(50) { Side::Lo } else { Side::Hi };
            let eager = eagerness[if side == Side::Lo { 0 } else { 1 }];
            if rng.chance(eager) {
                events.push((side, at));
            }
        }
        (zones, events)
    }

    /// The definition, computed the slow way from the full history: walk the
    /// calendar and count consecutive dates both acted on, as known at `now`.
    fn reference(zones: Zones, events: &[(Side, DateTime<Utc>)], now: DateTime<Utc>, limits: Limits) -> (i32, i32) {
        let dates_of = |side: Side| -> BTreeSet<NaiveDate> {
            events.iter().filter(|(s, _)| *s == side).map(|(_, at)| local_date(*at, zones.of(side))).collect()
        };
        let (lo, hi) = (dates_of(Side::Lo), dates_of(Side::Hi));
        let Some(first) = lo.iter().chain(hi.iter()).min().copied() else { return (0, 0) };
        let (mut count, mut best) = (0, 0);
        let mut date = first;
        loop {
            let (has_lo, has_hi) = (lo.contains(&date), hi.contains(&date));
            if has_lo && has_hi {
                count += 1;
                best = best.max(count);
            } else {
                let grace = if count > 0 { limits.grace } else { Duration::zero() };
                let lo_out = !has_lo && end_of_day(date, zones.lo) + grace <= now;
                let hi_out = !has_hi && end_of_day(date, zones.hi) + grace <= now;
                if lo_out || hi_out {
                    count = 0;
                } else if count > 0 || has_lo || has_hi {
                    // Still open: nothing after it is settled.
                    break;
                }
            }
            if date > local_date(now, zones.lo).max(local_date(now, zones.hi)) {
                break;
            }
            date = next_day(date);
        }
        (count, best)
    }

    #[test]
    fn the_machine_agrees_with_the_definition_for_random_histories_in_any_zones() {
        for seed in 0..600 {
            let mut rng = Rng(seed);
            let (zones, events) = history(&mut rng);
            let mut days = Days::default();
            for (index, (side, at)) in events.iter().enumerate() {
                days.act(*side, *at, zones, limits());
                let expected = reference(zones, &events[..=index], *at, limits());
                assert_eq!(
                    (days.count, days.best),
                    expected,
                    "seed {seed}, after event {index} ({side:?} at {at}) in {zones:?}"
                );
            }
            // Long after the last event everything is settled: no streak survives.
            let later = events.last().map_or(Utc::now(), |(_, at)| *at) + Duration::days(4);
            days.settle(later, zones, limits());
            assert_eq!(days.count, 0, "seed {seed}");
            assert_eq!(days.best, reference(zones, &events, later, limits()).1, "seed {seed}");
        }
    }

    #[test]
    fn redelivered_events_change_nothing() {
        for seed in 0..300 {
            let mut rng = Rng(seed ^ 0xfeed);
            let (zones, events) = history(&mut rng);
            let mut days = Days::default();
            let mut total_completed = 0;
            for (index, (side, at)) in events.iter().enumerate() {
                let steps = days.act(*side, *at, zones, limits());
                total_completed += steps.iter().filter(|step| matches!(step, Step::Completed { .. })).count();
                // The same event straight away again...
                let before = days.clone();
                assert!(
                    days.act(*side, *at, zones, limits()).is_empty(),
                    "seed {seed}: immediate redelivery of event {index} did something"
                );
                assert_eq!(days, before, "seed {seed}: immediate redelivery of event {index}");
                // ...and any earlier event, long after it was first handled.
                if index > 0 && rng.chance(40) {
                    let (old_side, old_at) = events[rng.below(index as u64) as usize];
                    assert!(
                        days.act(old_side, old_at, zones, limits()).is_empty(),
                        "seed {seed}: replaying an old event after event {index} did something"
                    );
                    assert_eq!(days, before, "seed {seed}: replaying an old event after event {index}");
                }
            }
            // Every completed date was reported exactly once.
            let dates_of = |side: Side| -> BTreeSet<NaiveDate> {
                events.iter().filter(|(s, _)| *s == side).map(|(_, at)| local_date(*at, zones.of(side))).collect()
            };
            let both = dates_of(Side::Lo).intersection(&dates_of(Side::Hi)).count();
            assert!(total_completed <= both, "seed {seed}");
        }
    }

    #[test]
    fn a_time_zone_never_shortens_anybodys_day() {
        // Whatever the two zones, if each acts once on every one of their own
        // local dates, the streak never breaks.
        for seed in 0..200 {
            let mut rng = Rng(seed ^ 0xbeef);
            let zones = Zones {
                lo: ZONES[rng.below(ZONES.len() as u64) as usize],
                hi: ZONES[rng.below(ZONES.len() as u64) as usize],
            };
            let first = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
            let mut events = Vec::new();
            for offset in 0..40 {
                let day = first + Duration::days(offset);
                for side in [Side::Lo, Side::Hi] {
                    // Some moment of that local day, from just after midnight to just before the next.
                    let start = crate::clock::start_of_day(day, zones.of(side));
                    let length = (end_of_day(day, zones.of(side)) - start).num_minutes();
                    events.push((side, start + Duration::minutes(rng.below(length as u64) as i64)));
                }
            }
            events.sort_by_key(|(_, at)| *at);
            let mut days = Days::default();
            for (side, at) in &events {
                let steps = days.act(*side, *at, zones, limits());
                assert!(
                    !steps.iter().any(|step| matches!(step, Step::Lost { .. })),
                    "seed {seed}: lost a streak in {zones:?}"
                );
            }
            assert_eq!(days.best, days.count, "seed {seed}");
            assert!(days.count >= 39, "seed {seed}: only {} days in {zones:?}", days.count);
        }
    }
}
