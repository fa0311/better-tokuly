//! Best-friend maths, free of I/O: how the interaction log turns into a
//! score, a weekly streak and relationship titles.
//!
//! The log is bucketed per pair, day and kind (`friend_interaction_days`).
//! A pair's score is the sum of each day's points, halved every
//! [`HALF_LIFE_DAYS`] days of age, over a [`WINDOW_DAYS`]-day window. Each
//! side ranks its friends by that score.
use chrono::{DateTime, NaiveDate, NaiveTime, Timelike as _, Utc, Weekday};

/// Days of history that count towards the score and the titles.
pub(crate) const WINDOW_DAYS: i64 = 30;
/// Age at which a day's points count half.
const HALF_LIFE_DAYS: f64 = 10.0;
/// Below this score a friend is not a best friend, however short the list.
pub(crate) const MIN_BEST_FRIEND_SCORE: f64 = 10.0;
/// Minutes two friends must spend together between midnight and
/// [`NIGHT_END_HOUR`] for the night to count.
pub(crate) const NIGHT_MINUTES_REQUIRED: i32 = 60;
const NIGHT_END_HOUR: u32 = 5;

/// What one row of `friend_interaction_days` counts. The numbers are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Bucket {
    Message = 1,
    Stamp = 2,
    Bump = 3,
    MapView = 4,
    MapReaction = 5,
    /// Minutes on a call.
    Call = 6,
    /// Minutes physically together.
    TimeTogether = 7,
    /// A night spent at the same place (derived from `TimeTogether`).
    Night = 8,
}

impl Bucket {
    pub(crate) fn code(self) -> i16 {
        self as i16
    }

    pub(crate) fn from_code(code: i16) -> Option<Self> {
        Some(match code {
            1 => Self::Message,
            2 => Self::Stamp,
            3 => Self::Bump,
            4 => Self::MapView,
            5 => Self::MapReaction,
            6 => Self::Call,
            7 => Self::TimeTogether,
            8 => Self::Night,
            _ => return None,
        })
    }

    /// `(points per unit, most points one side can earn per day, shared)`.
    ///
    /// Shared kinds (bumps, calls, time together) are things two people do
    /// together, so who "initiated" them is meaningless. For the others each
    /// side has its own daily cap, which keeps one chatty day or one spammy
    /// friend from dominating the ranking.
    fn scoring(self) -> (f64, f64, bool) {
        match self {
            Self::Message => (1.0, 40.0, false),
            Self::Stamp => (1.0, 20.0, false),
            Self::Bump => (15.0, 45.0, true),
            Self::MapView => (0.5, 4.0, false),
            Self::MapReaction => (2.0, 16.0, false),
            Self::Call => (1.0, 45.0, true),
            Self::TimeTogether => (0.25, 60.0, true),
            Self::Night => (25.0, 25.0, true),
        }
    }
}

/// One day of one kind of interaction between two users.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DayBucket {
    /// Whole days between that day and today (0 = today).
    pub age_days: i64,
    pub bucket: Bucket,
    /// Units initiated by the pair's `user_lo` / `user_hi`.
    pub lo: i64,
    pub hi: i64,
}

/// Undecayed points of one day bucket.
///
/// For one-directional kinds, what both sides did counts double and the
/// surplus of the more active side counts half: a friendship is mutual, and
/// someone who only receives should not get a "best friend" out of it.
fn day_points(bucket: Bucket, lo: i64, hi: i64) -> f64 {
    let (per_unit, daily_cap, shared) = bucket.scoring();
    let (lo, hi) = (lo.max(0) as f64 * per_unit, hi.max(0) as f64 * per_unit);
    if shared {
        return (lo + hi).min(daily_cap);
    }
    let (lo, hi) = (lo.min(daily_cap), hi.min(daily_cap));
    let mutual = lo.min(hi);
    let surplus = (lo - hi).abs();
    2.0 * mutual + 0.5 * surplus
}

fn decay(age_days: i64) -> f64 {
    0.5_f64.powf(age_days.max(0) as f64 / HALF_LIFE_DAYS)
}

/// What a pair did within the window.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct PairWindow {
    pub score: f64,
    /// Messages and stamps exchanged (`Friendship.interaction_score`).
    pub messages_and_stamps: i64,
    pub bumps: i64,
    pub nights: i64,
    pub minutes_together: i64,
}

/// Sums a pair's day buckets; buckets outside the window are ignored.
pub(crate) fn summarize(days: &[DayBucket]) -> PairWindow {
    let mut window = PairWindow::default();
    for day in days.iter().filter(|d| (0..WINDOW_DAYS).contains(&d.age_days)) {
        window.score += day_points(day.bucket, day.lo, day.hi) * decay(day.age_days);
        let units = day.lo.max(0) + day.hi.max(0);
        match day.bucket {
            Bucket::Message | Bucket::Stamp => window.messages_and_stamps += units,
            Bucket::Bump => window.bumps += units,
            Bucket::Night => window.nights += units,
            Bucket::TimeTogether => window.minutes_together += units,
            Bucket::MapView | Bucket::MapReaction | Bucket::Call => {}
        }
    }
    window
}

pub(crate) const TITLE_BFF: &str = "BFF";
pub(crate) const TITLE_MUTUAL_LOVE: &str = "MUTUAL_LOVE";
pub(crate) const TITLE_PARTY_PEOPLE: &str = "PARTY_PEOPLE";
pub(crate) const TITLE_BUMP_BUDDIES: &str = "BUMP_BUDDIES";
pub(crate) const TITLE_CHATTERBOX: &str = "CHATTERBOX";
pub(crate) const TITLE_ON_FIRE: &str = "ON_FIRE";
pub(crate) const TITLE_NEW_FRIENDS: &str = "NEW_FRIENDS";

/// Time together below this never earns `MUTUAL_LOVE`, even when it is the
/// most either side has.
pub(crate) const MUTUAL_LOVE_MIN_MINUTES: i64 = 180;

/// Titles that depend on the 30-day window; stored with the pair by the
/// ranking worker. `mutual_top_together` says each is the other's
/// most-time-together friend.
pub(crate) fn window_titles(window: &PairWindow, mutual_top_together: bool) -> Vec<&'static str> {
    let mut titles = Vec::new();
    if mutual_top_together && window.minutes_together >= MUTUAL_LOVE_MIN_MINUTES {
        titles.push(TITLE_MUTUAL_LOVE);
    }
    if window.nights >= 3 {
        titles.push(TITLE_PARTY_PEOPLE);
    }
    if window.bumps >= 5 {
        titles.push(TITLE_BUMP_BUDDIES);
    }
    if window.messages_and_stamps >= 200 {
        titles.push(TITLE_CHATTERBOX);
    }
    titles
}

/// The full title list shown to a friend: the stored window titles plus the
/// ones that follow from facts known at read time.
pub(crate) fn all_titles(stored: &[String], mutual_closest: bool, streak_weeks: i32, friends_since: DateTime<Utc>, now: DateTime<Utc>) -> Vec<String> {
    let mut titles = Vec::with_capacity(stored.len() + 3);
    if mutual_closest {
        titles.push(TITLE_BFF.to_owned());
    }
    titles.extend(stored.iter().cloned());
    if streak_weeks >= 4 {
        titles.push(TITLE_ON_FIRE.to_owned());
    }
    if now - friends_since < chrono::Duration::days(7) {
        titles.push(TITLE_NEW_FRIENDS.to_owned());
    }
    titles
}

/// Monday of the week `day` falls in; weeks are the unit of streaks.
pub(crate) fn week_start(day: NaiveDate) -> NaiveDate {
    day.week(Weekday::Mon).first_day()
}

/// The streak as it stands today: a streak survives while its last counted
/// week is this week or the previous one (the current week may still get its
/// interaction), and is broken after a whole week without any.
pub(crate) fn effective_streak(streak_weeks: i32, streak_week: Option<NaiveDate>, today: NaiveDate) -> i32 {
    match streak_week {
        Some(week) if week >= week_start(today) - chrono::Duration::days(7) => streak_weeks,
        _ => 0,
    }
}

/// Whether a local time of day falls into the hours that make a night together.
pub(crate) fn is_night(local: NaiveTime) -> bool {
    local.hour() < NIGHT_END_HOUR
}

/// Progress towards counting one night for a pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NightProgress {
    pub minutes: i32,
    pub date: NaiveDate,
    /// These minutes pushed the night over the threshold: count it now.
    pub completed: bool,
}

/// Adds `added` night minutes on `night` to what was accumulated before
/// (`previous`: minutes and the night they belong to). Minutes from another
/// night do not carry over, and a night is completed exactly once.
pub(crate) fn night_progress(previous: (i32, Option<NaiveDate>), night: NaiveDate, added: i32) -> NightProgress {
    let before = if previous.1 == Some(night) { previous.0 } else { 0 };
    let minutes = before.saturating_add(added.max(0));
    NightProgress { minutes, date: night, completed: before < NIGHT_MINUTES_REQUIRED && minutes >= NIGHT_MINUTES_REQUIRED }
}

/// Whole days from `day` to `today` (negative if `day` is in the future).
pub(crate) fn age_days(day: NaiveDate, today: NaiveDate) -> i64 {
    (today - day).num_days()
}

/// First day inside the scoring window ending today.
pub(crate) fn window_start(today: NaiveDate) -> NaiveDate {
    today - chrono::Duration::days(WINDOW_DAYS - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(age_days: i64, bucket: Bucket, lo: i64, hi: i64) -> DayBucket {
        DayBucket { age_days, bucket, lo, hi }
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn mutual_interaction_outscores_one_sided() {
        // Twenty messages either way: a conversation counts twice as much
        // as a monologue.
        let mutual = summarize(&[day(0, Bucket::Message, 10, 10)]).score;
        let one_sided = summarize(&[day(0, Bucket::Message, 20, 0)]).score;
        assert_eq!(mutual, 20.0);
        assert_eq!(one_sided, 10.0);
    }

    #[test]
    fn each_side_is_capped_per_day() {
        let spam = summarize(&[day(0, Bucket::Message, 10_000, 0)]).score;
        assert_eq!(spam, 20.0, "one side: capped at 40 points, counted half");
        let busy = summarize(&[day(0, Bucket::Message, 10_000, 10_000)]).score;
        assert_eq!(busy, 80.0);
        // Shared kinds are capped on their sum, whoever reported them.
        assert_eq!(summarize(&[day(0, Bucket::Bump, 100, 0)]).score, 45.0);
        assert_eq!(summarize(&[day(0, Bucket::TimeTogether, 60, 60)]).score, 30.0);
    }

    #[test]
    fn older_days_count_less_and_leave_the_window() {
        let today = summarize(&[day(0, Bucket::Bump, 1, 0)]).score;
        let ten_days_ago = summarize(&[day(10, Bucket::Bump, 1, 0)]).score;
        assert_eq!(today, 15.0);
        assert!((ten_days_ago - 7.5).abs() < 1e-9, "half-life is ten days");
        assert_eq!(summarize(&[day(WINDOW_DAYS, Bucket::Bump, 1, 0)]).score, 0.0);
        assert_eq!(summarize(&[day(-1, Bucket::Bump, 1, 0)]).score, 0.0, "future days are ignored");
    }

    #[test]
    fn window_totals_feed_titles() {
        let window = summarize(&[
            day(0, Bucket::Message, 120, 60),
            day(1, Bucket::Stamp, 10, 15),
            day(2, Bucket::Bump, 3, 2),
            day(3, Bucket::Night, 3, 0),
            day(4, Bucket::TimeTogether, 100, 100),
        ]);
        assert_eq!((window.messages_and_stamps, window.bumps, window.nights, window.minutes_together), (205, 5, 3, 200));
        assert_eq!(window_titles(&window, true), vec![TITLE_MUTUAL_LOVE, TITLE_PARTY_PEOPLE, TITLE_BUMP_BUDDIES, TITLE_CHATTERBOX]);
        assert_eq!(window_titles(&window, false), vec![TITLE_PARTY_PEOPLE, TITLE_BUMP_BUDDIES, TITLE_CHATTERBOX]);
        let quiet = summarize(&[day(0, Bucket::TimeTogether, 30, 0)]);
        assert!(window_titles(&quiet, true).is_empty(), "half an hour together is not mutual love");
    }

    #[test]
    fn read_time_titles_are_added_around_the_stored_ones() {
        let now = Utc::now();
        let stored = vec![TITLE_CHATTERBOX.to_owned()];
        let old = now - chrono::Duration::days(100);
        assert_eq!(all_titles(&stored, true, 4, old, now), vec![TITLE_BFF, TITLE_CHATTERBOX, TITLE_ON_FIRE]);
        assert_eq!(all_titles(&[], false, 3, now - chrono::Duration::days(2), now), vec![TITLE_NEW_FRIENDS]);
    }

    #[test]
    fn weeks_start_on_monday() {
        // 2026-11-20 is a Friday.
        assert_eq!(week_start(date(2026, 11, 20)), date(2026, 11, 16));
        assert_eq!(week_start(date(2026, 11, 16)), date(2026, 11, 16));
        assert_eq!(week_start(date(2026, 11, 22)), date(2026, 11, 16));
    }

    #[test]
    fn streaks_survive_the_running_week_and_break_after_a_missed_one() {
        let today = date(2026, 11, 20);
        assert_eq!(effective_streak(5, Some(date(2026, 11, 16)), today), 5, "counted this week");
        assert_eq!(effective_streak(5, Some(date(2026, 11, 9)), today), 5, "last week counted; this week is still open");
        assert_eq!(effective_streak(5, Some(date(2026, 11, 2)), today), 0, "a whole week was missed");
        assert_eq!(effective_streak(5, None, today), 0);
    }

    #[test]
    fn a_night_is_counted_once_when_it_reaches_the_threshold() {
        let night = date(2026, 11, 20);
        let first = night_progress((0, None), night, 40);
        assert_eq!((first.minutes, first.completed), (40, false));
        let second = night_progress((first.minutes, Some(first.date)), night, 30);
        assert_eq!((second.minutes, second.completed), (70, true));
        let third = night_progress((second.minutes, Some(second.date)), night, 30);
        assert!(!third.completed, "already counted");
        let next_night = night_progress((third.minutes, Some(third.date)), date(2026, 11, 21), 30);
        assert_eq!((next_night.minutes, next_night.completed), (30, false), "minutes do not carry over");
    }

    #[test]
    fn night_hours_are_after_midnight() {
        assert!(is_night(NaiveTime::from_hms_opt(0, 0, 0).unwrap()));
        assert!(is_night(NaiveTime::from_hms_opt(4, 59, 59).unwrap()));
        assert!(!is_night(NaiveTime::from_hms_opt(5, 0, 0).unwrap()));
        assert!(!is_night(NaiveTime::from_hms_opt(23, 30, 0).unwrap()));
    }

    #[test]
    fn bucket_codes_roundtrip() {
        for bucket in [Bucket::Message, Bucket::Stamp, Bucket::Bump, Bucket::MapView, Bucket::MapReaction, Bucket::Call, Bucket::TimeTogether, Bucket::Night] {
            assert_eq!(Bucket::from_code(bucket.code()), Some(bucket));
        }
        assert_eq!(Bucket::from_code(0), None);
    }
}
