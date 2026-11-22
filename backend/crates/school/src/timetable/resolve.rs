//! From a term's weekly grid to what actually happens on a date.
//!
//! Pure functions over a term loaded into memory ([`TermData`]): which week
//! (A or B) a date belongs to, whether there is school at all (holidays,
//! class days, weekdays without classes), the classes of the grid for that
//! day, and the one-off changes on top.
use chrono::{Duration, NaiveDate};
use tokuly_core::prelude::*;
use tokuly_proto::school::{ChangeKind, ClassStatus, DayKind, DayOverrideKind, WeekPattern};

use super::{holidays, layout::Layout};
use crate::local;

pub(crate) const EVERY: i16 = WeekPattern::WEEK_PATTERN_EVERY as i16;
pub(crate) const WEEK_A: i16 = WeekPattern::WEEK_PATTERN_A as i16;
pub(crate) const WEEK_B: i16 = WeekPattern::WEEK_PATTERN_B as i16;

/// A term's own fields, as the resolver needs them.
#[derive(Debug, Clone)]
pub(crate) struct TermInfo {
    pub id: Uuid,
    pub name: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub is_current: bool,
    pub layout: Layout,
    pub alternating_weeks: bool,
    pub week_a_date: Option<NaiveDate>,
    pub observe_public_holidays: bool,
}

impl TermInfo {
    pub fn covers(&self, date: NaiveDate) -> bool {
        self.start_date <= date && date <= self.end_date
    }
}

#[derive(Debug, Clone)]
pub(crate) struct EntryData {
    pub id: Uuid,
    pub weekday: i16,
    pub period: i16,
    pub week_pattern: i16,
    pub subject_id: Uuid,
    pub teacher: String,
    pub room: String,
    pub color: String,
    pub memo: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ChangeData {
    pub id: Uuid,
    pub date: NaiveDate,
    pub period: i16,
    pub kind: i16,
    pub subject_id: Option<Uuid>,
    pub teacher: String,
    pub room: String,
    pub memo: String,
    pub to_date: Option<NaiveDate>,
    pub to_period: Option<i16>,
}

#[derive(Debug, Clone)]
pub(crate) struct OverrideData {
    pub id: Uuid,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub kind: i16,
    pub name: String,
    pub follows_weekday: Option<i16>,
    pub week_pattern: Option<i16>,
}

impl OverrideData {
    fn covers(&self, date: NaiveDate) -> bool {
        self.start_date <= date && date <= self.end_date
    }

    fn days(&self) -> i64 {
        (self.end_date - self.start_date).num_days()
    }
}

/// A term with everything that decides its days.
#[derive(Debug, Clone)]
pub(crate) struct TermData {
    pub info: TermInfo,
    pub entries: Vec<EntryData>,
    pub changes: Vec<ChangeData>,
    pub overrides: Vec<OverrideData>,
}

/// A class on a date. Teacher, room and colour are the entry's or change's
/// own; empty means "the subject's".
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedClass {
    pub date: NaiveDate,
    pub period: i32,
    pub start_minute: i32,
    pub end_minute: i32,
    pub status: ClassStatus,
    pub subject_id: Uuid,
    pub original_subject_id: Option<Uuid>,
    pub teacher: String,
    pub room: String,
    pub color: String,
    pub memo: String,
    pub entry_id: Option<Uuid>,
    pub change_id: Option<Uuid>,
    /// Where a moved class went to or came from.
    pub other: Option<(NaiveDate, i32)>,
}

impl ResolvedClass {
    /// Cancelled and moved-away classes are listed but do not happen.
    pub fn takes_place(&self) -> bool {
        !matches!(self.status, ClassStatus::CLASS_STATUS_CANCELLED | ClassStatus::CLASS_STATUS_MOVED_AWAY)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedDay {
    pub date: NaiveDate,
    pub kind: DayKind,
    /// Name of the holiday or override in effect.
    pub label: String,
    /// `WEEK_A` / `WEEK_B` in an alternating term, `EVERY` otherwise.
    pub week_pattern: i16,
    /// The weekday whose timetable the day follows (1 = Monday).
    pub follows_weekday: i32,
    pub classes: Vec<ResolvedClass>,
}

/// What kind of day a date is before any one-off change.
struct DayShape {
    kind: DayKind,
    label: String,
    week_pattern: i16,
    follows_weekday: i32,
}

/// Among the overrides of the given kinds covering `date`: the shortest
/// range, then the newest (IDs are time-ordered).
fn winning_override<'a>(term: &'a TermData, date: NaiveDate, kinds: &[DayOverrideKind]) -> Option<&'a OverrideData> {
    term.overrides
        .iter()
        .filter(|o| o.covers(date) && kinds.iter().any(|kind| *kind as i16 == o.kind))
        .min_by_key(|o| (o.days(), std::cmp::Reverse(o.id)))
}

/// Week A or B by alternation from the term's reference week.
fn alternation(week_a_date: NaiveDate, date: NaiveDate) -> i16 {
    let weeks = (local::week_start(date) - local::week_start(week_a_date)).num_days() / 7;
    if weeks.rem_euclid(2) == 0 { WEEK_A } else { WEEK_B }
}

fn day_shape(term: &TermData, date: NaiveDate) -> DayShape {
    let info = &term.info;
    let own_weekday = local::weekday_of(date);
    let day_rule = winning_override(
        term,
        date,
        &[DayOverrideKind::DAY_OVERRIDE_KIND_HOLIDAY, DayOverrideKind::DAY_OVERRIDE_KIND_CLASS_DAY],
    );
    let class_day = day_rule.filter(|o| o.kind == DayOverrideKind::DAY_OVERRIDE_KIND_CLASS_DAY as i16);

    let week_pattern = if !info.alternating_weeks {
        EVERY
    } else if let Some(pinned) = class_day.and_then(|o| o.week_pattern) {
        pinned
    } else if let Some(pinned) =
        winning_override(term, date, &[DayOverrideKind::DAY_OVERRIDE_KIND_WEEK_PATTERN]).and_then(|o| o.week_pattern)
    {
        pinned
    } else {
        alternation(info.week_a_date.unwrap_or(info.start_date), date)
    };

    let (kind, label, follows_weekday) = match day_rule {
        Some(rule) if rule.kind == DayOverrideKind::DAY_OVERRIDE_KIND_HOLIDAY as i16 => {
            let label = if rule.name.is_empty() { "休み".to_owned() } else { rule.name.clone() };
            (DayKind::DAY_KIND_HOLIDAY, label, own_weekday)
        }
        Some(rule) => (
            DayKind::DAY_KIND_SCHOOL_DAY,
            rule.name.clone(),
            rule.follows_weekday.map(i32::from).unwrap_or(own_weekday),
        ),
        None => match holidays::public_holiday(date).filter(|_| info.observe_public_holidays) {
            Some(holiday) => (DayKind::DAY_KIND_HOLIDAY, holiday.to_owned(), own_weekday),
            None if info.layout.has_weekday(own_weekday) => (DayKind::DAY_KIND_SCHOOL_DAY, String::new(), own_weekday),
            None => (DayKind::DAY_KIND_DAY_OFF, String::new(), own_weekday),
        },
    };
    DayShape { kind, label, week_pattern, follows_weekday }
}

/// The classes the weekly grid puts on `date`, before one-off changes.
fn grid_classes(term: &TermData, date: NaiveDate, shape: &DayShape) -> Vec<ResolvedClass> {
    if shape.kind != DayKind::DAY_KIND_SCHOOL_DAY {
        return Vec::new();
    }
    let mut classes: Vec<ResolvedClass> = term
        .entries
        .iter()
        .filter(|entry| entry.weekday as i32 == shape.follows_weekday)
        .filter(|entry| entry.week_pattern == EVERY || entry.week_pattern == shape.week_pattern)
        .filter_map(|entry| {
            let period = term.info.layout.period(entry.period as i32)?;
            Some(ResolvedClass {
                date,
                period: period.number,
                start_minute: period.start_minute,
                end_minute: period.end_minute,
                status: ClassStatus::CLASS_STATUS_SCHEDULED,
                subject_id: entry.subject_id,
                original_subject_id: None,
                teacher: entry.teacher.clone(),
                room: entry.room.clone(),
                color: entry.color.clone(),
                memo: entry.memo.clone(),
                entry_id: Some(entry.id),
                change_id: None,
                other: None,
            })
        })
        .collect();
    classes.sort_by_key(|class| class.period);
    classes
}

/// The class the weekly grid puts into a slot on a date, if any. One-off
/// changes that cancel, move or replace a class need one to act on.
pub(crate) fn grid_class(term: &TermData, date: NaiveDate, period: i32) -> Option<ResolvedClass> {
    if !term.info.covers(date) {
        return None;
    }
    let shape = day_shape(term, date);
    grid_classes(term, date, &shape).into_iter().find(|class| class.period == period)
}

/// Resolves one day of a term. `date` must be inside the term.
pub(crate) fn resolve_day(term: &TermData, date: NaiveDate) -> ResolvedDay {
    let shape = day_shape(term, date);
    let mut classes = grid_classes(term, date, &shape);

    for change in term.changes.iter().filter(|change| change.date == date) {
        let slot = classes.iter().position(|class| class.period == change.period as i32);
        match (change.kind, slot) {
            (kind, Some(index)) if kind == ChangeKind::CHANGE_KIND_CANCELLED as i16 => {
                let class = &mut classes[index];
                class.status = ClassStatus::CLASS_STATUS_CANCELLED;
                class.change_id = Some(change.id);
                apply_memo(class, &change.memo);
            }
            (kind, Some(index)) if kind == ChangeKind::CHANGE_KIND_MOVED as i16 => {
                let class = &mut classes[index];
                class.status = ClassStatus::CLASS_STATUS_MOVED_AWAY;
                class.change_id = Some(change.id);
                class.other = change.to_date.zip(change.to_period.map(i32::from));
                apply_memo(class, &change.memo);
            }
            (kind, Some(index)) if kind == ChangeKind::CHANGE_KIND_REPLACED as i16 => {
                let class = &mut classes[index];
                class.status = ClassStatus::CLASS_STATUS_REPLACED;
                class.change_id = Some(change.id);
                match change.subject_id.filter(|subject| *subject != class.subject_id) {
                    // Another subject: nothing of the usual class carries over.
                    Some(subject) => {
                        class.original_subject_id = Some(class.subject_id);
                        class.subject_id = subject;
                        class.teacher = change.teacher.clone();
                        class.room = change.room.clone();
                        class.color = String::new();
                    }
                    // The same class somewhere else or with someone else.
                    None => {
                        if !change.teacher.is_empty() {
                            class.teacher = change.teacher.clone();
                        }
                        if !change.room.is_empty() {
                            class.room = change.room.clone();
                        }
                    }
                }
                apply_memo(class, &change.memo);
            }
            (kind, _) if kind == ChangeKind::CHANGE_KIND_EXTRA as i16 => {
                let (Some(subject_id), Some(period)) =
                    (change.subject_id, term.info.layout.period(change.period as i32))
                else {
                    continue;
                };
                classes.push(ResolvedClass {
                    date,
                    period: period.number,
                    start_minute: period.start_minute,
                    end_minute: period.end_minute,
                    status: ClassStatus::CLASS_STATUS_EXTRA,
                    subject_id,
                    original_subject_id: None,
                    teacher: change.teacher.clone(),
                    room: change.room.clone(),
                    color: String::new(),
                    memo: change.memo.clone(),
                    entry_id: None,
                    change_id: Some(change.id),
                    other: None,
                });
            }
            // The grid changed under the change: there is no class to act on anymore.
            _ => {}
        }
    }

    // Classes moved here from another slot (possibly of the same day).
    let moved = ChangeKind::CHANGE_KIND_MOVED as i16;
    for change in term.changes.iter().filter(|change| change.kind == moved && change.to_date == Some(date)) {
        let (Some(to_period), Some(source)) = (
            change.to_period.and_then(|p| term.info.layout.period(p as i32)),
            grid_class(term, change.date, change.period as i32),
        ) else {
            continue;
        };
        classes.push(ResolvedClass {
            date,
            period: to_period.number,
            start_minute: to_period.start_minute,
            end_minute: to_period.end_minute,
            status: ClassStatus::CLASS_STATUS_MOVED_HERE,
            change_id: Some(change.id),
            other: Some((change.date, change.period as i32)),
            memo: if change.memo.is_empty() { source.memo.clone() } else { change.memo.clone() },
            ..source
        });
    }

    classes.sort_by_key(|class| (class.period, !class.takes_place()));
    ResolvedDay {
        date,
        kind: shape.kind,
        label: shape.label,
        week_pattern: shape.week_pattern,
        follows_weekday: shape.follows_weekday,
        classes,
    }
}

fn apply_memo(class: &mut ResolvedClass, memo: &str) {
    if !memo.is_empty() {
        class.memo = memo.to_owned();
    }
}

/// A date no term covers.
pub(crate) fn out_of_term(date: NaiveDate) -> ResolvedDay {
    ResolvedDay {
        date,
        kind: DayKind::DAY_KIND_OUT_OF_TERM,
        label: String::new(),
        week_pattern: EVERY,
        follows_weekday: local::weekday_of(date),
        classes: Vec::new(),
    }
}

/// The term that decides `date`: among those covering it, the current one,
/// else the one starting last (newest first on a tie).
pub(crate) fn term_for<'a>(terms: impl IntoIterator<Item = &'a TermInfo>, date: NaiveDate) -> Option<&'a TermInfo> {
    terms.into_iter().filter(|term| term.covers(date)).max_by_key(|term| (term.is_current, term.start_date, term.id))
}

/// The dates from `start` to `end`, inclusive.
pub(crate) fn dates(start: NaiveDate, end: NaiveDate) -> impl Iterator<Item = NaiveDate> {
    let days = (end - start).num_days().max(-1);
    (0..=days).map(move |offset| start + Duration::days(offset))
}

#[cfg(test)]
mod tests {
    use tokuly_proto::school::LayoutPreset;

    use super::*;
    use crate::timetable::layout;

    fn date(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).unwrap()
    }

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    const MATH: Uuid = Uuid::from_u128(101);
    const ENGLISH: Uuid = Uuid::from_u128(102);
    const PE: Uuid = Uuid::from_u128(103);
    const ART: Uuid = Uuid::from_u128(104);

    fn entry(n: u128, weekday: i16, period: i16, week_pattern: i16, subject_id: Uuid) -> EntryData {
        EntryData {
            id: id(n),
            weekday,
            period,
            week_pattern,
            subject_id,
            teacher: String::new(),
            room: String::new(),
            color: String::new(),
            memo: String::new(),
        }
    }

    fn change(n: u128, on: NaiveDate, period: i16, kind: ChangeKind) -> ChangeData {
        ChangeData {
            id: id(n),
            date: on,
            period,
            kind: kind as i16,
            subject_id: None,
            teacher: String::new(),
            room: String::new(),
            memo: String::new(),
            to_date: None,
            to_period: None,
        }
    }

    fn day_override(n: u128, start: NaiveDate, end: NaiveDate, kind: DayOverrideKind) -> OverrideData {
        OverrideData {
            id: id(n),
            start_date: start,
            end_date: end,
            kind: kind as i16,
            name: String::new(),
            follows_weekday: None,
            week_pattern: None,
        }
    }

    /// First term of 2026 (April 8 to July 20), Monday to Friday, six periods:
    /// Monday: 1 math, 2 English; Tuesday: 1 PE; Wednesday: 1 math (week A) / art (week B), 3 English.
    fn term() -> TermData {
        TermData {
            info: TermInfo {
                id: id(1),
                name: "1学期".into(),
                start_date: date(4, 8),
                end_date: date(7, 20),
                is_current: true,
                layout: layout::preset(LayoutPreset::LAYOUT_PRESET_HIGH_SCHOOL_6).unwrap(),
                alternating_weeks: true,
                week_a_date: Some(date(4, 8)),
                observe_public_holidays: true,
            },
            entries: vec![
                entry(11, 1, 1, EVERY, MATH),
                entry(12, 1, 2, EVERY, ENGLISH),
                entry(13, 2, 1, EVERY, PE),
                entry(14, 3, 1, WEEK_A, MATH),
                entry(15, 3, 1, WEEK_B, ART),
                entry(16, 3, 3, EVERY, ENGLISH),
            ],
            changes: Vec::new(),
            overrides: Vec::new(),
        }
    }

    fn subjects(day: &ResolvedDay) -> Vec<(i32, Uuid, ClassStatus)> {
        day.classes.iter().map(|class| (class.period, class.subject_id, class.status)).collect()
    }

    use ClassStatus::*;

    #[test]
    fn a_plain_school_day_follows_the_grid() {
        let term = term();
        let monday = resolve_day(&term, date(4, 13));
        assert_eq!(monday.kind, DayKind::DAY_KIND_SCHOOL_DAY);
        assert_eq!(subjects(&monday), vec![(1, MATH, CLASS_STATUS_SCHEDULED), (2, ENGLISH, CLASS_STATUS_SCHEDULED)]);
        let first = &monday.classes[0];
        assert_eq!((first.start_minute, first.end_minute, first.entry_id), (530, 580, Some(id(11))));
        // Thursday has no classes in the grid but is a school day.
        let thursday = resolve_day(&term, date(4, 16));
        assert_eq!((thursday.kind, thursday.classes.len()), (DayKind::DAY_KIND_SCHOOL_DAY, 0));
    }

    #[test]
    fn weeks_alternate_from_the_reference_week() {
        let term = term();
        // April 8 is in week A; the following Wednesdays alternate.
        for (wednesday, pattern, first) in [
            (date(4, 8), WEEK_A, MATH),
            (date(4, 15), WEEK_B, ART),
            (date(4, 22), WEEK_A, MATH),
            (date(7, 15), WEEK_A, MATH),
        ] {
            let day = resolve_day(&term, wednesday);
            assert_eq!(day.week_pattern, pattern, "{wednesday}");
            assert_eq!(
                subjects(&day),
                vec![(1, first, CLASS_STATUS_SCHEDULED), (3, ENGLISH, CLASS_STATUS_SCHEDULED)],
                "{wednesday}"
            );
        }
        // Weeks before the reference week alternate backwards.
        assert_eq!(alternation(date(4, 8), date(4, 1)), WEEK_B);
        assert_eq!(alternation(date(4, 8), date(3, 25)), WEEK_A);
        // Sunday still belongs to the week that started on Monday.
        assert_eq!(alternation(date(4, 8), date(4, 12)), WEEK_A);
        assert_eq!(alternation(date(4, 8), date(4, 13)), WEEK_B);

        // Without alternation every entry counts and the pattern is EVERY.
        let mut plain = term;
        plain.info.alternating_weeks = false;
        plain.entries.retain(|entry| entry.week_pattern == EVERY);
        assert_eq!(resolve_day(&plain, date(4, 15)).week_pattern, EVERY);
    }

    #[test]
    fn weekends_and_public_holidays_are_free() {
        let mut term = term();
        assert_eq!(resolve_day(&term, date(4, 11)).kind, DayKind::DAY_KIND_DAY_OFF, "Saturday");
        let showa = resolve_day(&term, date(4, 29));
        assert_eq!((showa.kind, showa.label.as_str(), showa.classes.len()), (DayKind::DAY_KIND_HOLIDAY, "昭和の日", 0));
        // 5/6 is a substitute holiday (a Wednesday).
        assert_eq!(resolve_day(&term, date(5, 6)).label, "振替休日");

        // A university that holds classes on public holidays.
        term.info.observe_public_holidays = false;
        assert_eq!(resolve_day(&term, date(4, 29)).kind, DayKind::DAY_KIND_SCHOOL_DAY);

        // A school with Saturday classes.
        term.info.layout.weekdays.push(6);
        term.entries.push(entry(17, 6, 1, EVERY, ART));
        let saturday = resolve_day(&term, date(4, 11));
        assert_eq!(subjects(&saturday), vec![(1, ART, CLASS_STATUS_SCHEDULED)]);
    }

    #[test]
    fn overrides_make_holidays_class_days_and_pinned_weeks() {
        let mut term = term();
        // 創立記念日 on a Monday.
        term.overrides.push(OverrideData {
            name: "創立記念日".into(),
            ..day_override(21, date(4, 20), date(4, 20), DayOverrideKind::DAY_OVERRIDE_KIND_HOLIDAY)
        });
        let founding = resolve_day(&term, date(4, 20));
        assert_eq!(
            (founding.kind, founding.label.as_str(), founding.classes.len()),
            (DayKind::DAY_KIND_HOLIDAY, "創立記念日", 0)
        );

        // A holiday without a name still says it is one.
        term.overrides.push(day_override(22, date(4, 21), date(4, 21), DayOverrideKind::DAY_OVERRIDE_KIND_HOLIDAY));
        assert_eq!(resolve_day(&term, date(4, 21)).label, "休み");

        // 月曜授業 on the public holiday 4/29 (a Wednesday).
        term.overrides.push(OverrideData {
            name: "月曜授業".into(),
            follows_weekday: Some(1),
            ..day_override(23, date(4, 29), date(4, 29), DayOverrideKind::DAY_OVERRIDE_KIND_CLASS_DAY)
        });
        let swapped = resolve_day(&term, date(4, 29));
        assert_eq!(
            (swapped.kind, swapped.follows_weekday, swapped.label.as_str()),
            (DayKind::DAY_KIND_SCHOOL_DAY, 1, "月曜授業")
        );
        assert_eq!(subjects(&swapped), vec![(1, MATH, CLASS_STATUS_SCHEDULED), (2, ENGLISH, CLASS_STATUS_SCHEDULED)]);

        // A Saturday class day follows its own weekday: nothing in the grid, but school.
        term.overrides.push(day_override(24, date(5, 9), date(5, 9), DayOverrideKind::DAY_OVERRIDE_KIND_CLASS_DAY));
        assert_eq!(resolve_day(&term, date(5, 9)).kind, DayKind::DAY_KIND_SCHOOL_DAY);

        // The week of 4/13 would be B; pin it to A.
        term.overrides.push(OverrideData {
            week_pattern: Some(WEEK_A),
            ..day_override(25, date(4, 13), date(4, 17), DayOverrideKind::DAY_OVERRIDE_KIND_WEEK_PATTERN)
        });
        assert_eq!(subjects(&resolve_day(&term, date(4, 15)))[0].1, MATH);
        // A class day can pin its own week whatever the range says.
        term.overrides.push(OverrideData {
            week_pattern: Some(WEEK_B),
            ..day_override(26, date(4, 15), date(4, 15), DayOverrideKind::DAY_OVERRIDE_KIND_CLASS_DAY)
        });
        assert_eq!(subjects(&resolve_day(&term, date(4, 15)))[0].1, ART);
    }

    #[test]
    fn the_shortest_override_wins_then_the_newest() {
        let mut term = term();
        // Summer holidays with one class day (登校日) inside.
        term.overrides.push(OverrideData {
            name: "夏休み".into(),
            ..day_override(31, date(7, 1), date(7, 20), DayOverrideKind::DAY_OVERRIDE_KIND_HOLIDAY)
        });
        term.overrides.push(OverrideData {
            name: "登校日".into(),
            ..day_override(32, date(7, 6), date(7, 6), DayOverrideKind::DAY_OVERRIDE_KIND_CLASS_DAY)
        });
        assert_eq!(resolve_day(&term, date(7, 3)).kind, DayKind::DAY_KIND_HOLIDAY);
        let school_day = resolve_day(&term, date(7, 6));
        assert_eq!((school_day.kind, school_day.label.as_str()), (DayKind::DAY_KIND_SCHOOL_DAY, "登校日"));
        assert_eq!(school_day.classes.len(), 2, "a Monday");

        // Two rules for the same single day: the newer one counts.
        term.overrides.push(OverrideData {
            name: "台風".into(),
            ..day_override(33, date(7, 6), date(7, 6), DayOverrideKind::DAY_OVERRIDE_KIND_HOLIDAY)
        });
        assert_eq!(resolve_day(&term, date(7, 6)).label, "台風");
    }

    #[test]
    fn one_off_changes_cancel_replace_and_add_classes() {
        let mut term = term();
        let monday = date(4, 13);
        term.changes
            .push(ChangeData {
                memo: "先生出張".into(), ..change(41, monday, 1, ChangeKind::CHANGE_KIND_CANCELLED)
            });
        term.changes.push(ChangeData {
            subject_id: Some(PE),
            room: "体育館".into(),
            ..change(42, monday, 2, ChangeKind::CHANGE_KIND_REPLACED)
        });
        term.changes.push(ChangeData { subject_id: Some(ART), ..change(43, monday, 5, ChangeKind::CHANGE_KIND_EXTRA) });
        let day = resolve_day(&term, monday);
        assert_eq!(
            subjects(&day),
            vec![(1, MATH, CLASS_STATUS_CANCELLED), (2, PE, CLASS_STATUS_REPLACED), (5, ART, CLASS_STATUS_EXTRA)]
        );
        assert_eq!((day.classes[0].memo.as_str(), day.classes[0].change_id), ("先生出張", Some(id(41))));
        assert!(!day.classes[0].takes_place());
        assert_eq!((day.classes[1].original_subject_id, day.classes[1].room.as_str()), (Some(ENGLISH), "体育館"));
        assert_eq!((day.classes[2].start_minute, day.classes[2].entry_id), (13 * 60 + 30, None));
        // The week after is untouched.
        assert_eq!(
            subjects(&resolve_day(&term, date(4, 20))),
            vec![(1, MATH, CLASS_STATUS_SCHEDULED), (2, ENGLISH, CLASS_STATUS_SCHEDULED)]
        );
    }

    #[test]
    fn a_replacement_without_a_subject_is_a_room_change() {
        let mut term = term();
        term.entries[0].teacher = "田中".into();
        term.entries[0].room = "2-A".into();
        term.changes.push(ChangeData {
            room: "視聴覚室".into(),
            ..change(44, date(4, 13), 1, ChangeKind::CHANGE_KIND_REPLACED)
        });
        let class = &resolve_day(&term, date(4, 13)).classes[0];
        assert_eq!((class.status, class.subject_id, class.original_subject_id), (CLASS_STATUS_REPLACED, MATH, None));
        assert_eq!((class.teacher.as_str(), class.room.as_str()), ("田中", "視聴覚室"));
    }

    #[test]
    fn a_moved_class_leaves_one_slot_and_appears_in_another() {
        let mut term = term();
        // Monday's first period moves to Thursday's fourth.
        term.changes.push(ChangeData {
            to_date: Some(date(4, 16)),
            to_period: Some(4),
            ..change(45, date(4, 13), 1, ChangeKind::CHANGE_KIND_MOVED)
        });
        let monday = resolve_day(&term, date(4, 13));
        assert_eq!(subjects(&monday), vec![(1, MATH, CLASS_STATUS_MOVED_AWAY), (2, ENGLISH, CLASS_STATUS_SCHEDULED)]);
        assert_eq!(monday.classes[0].other, Some((date(4, 16), 4)));
        let thursday = resolve_day(&term, date(4, 16));
        assert_eq!(subjects(&thursday), vec![(4, MATH, CLASS_STATUS_MOVED_HERE)]);
        let moved = &thursday.classes[0];
        assert_eq!(
            (moved.other, moved.start_minute, moved.entry_id, moved.change_id),
            (Some((date(4, 13), 1)), 11 * 60 + 50, Some(id(11)), Some(id(45)))
        );

        // A class can move within its own day, and onto a holiday.
        term.changes.push(ChangeData {
            to_date: Some(date(4, 14)),
            to_period: Some(6),
            ..change(46, date(4, 14), 1, ChangeKind::CHANGE_KIND_MOVED)
        });
        assert_eq!(
            subjects(&resolve_day(&term, date(4, 14))),
            vec![(1, PE, CLASS_STATUS_MOVED_AWAY), (6, PE, CLASS_STATUS_MOVED_HERE)]
        );
        term.changes.push(ChangeData {
            to_date: Some(date(4, 29)),
            to_period: Some(2),
            ..change(47, date(4, 20), 2, ChangeKind::CHANGE_KIND_MOVED)
        });
        let holiday = resolve_day(&term, date(4, 29));
        assert_eq!(
            (holiday.kind, subjects(&holiday)),
            (DayKind::DAY_KIND_HOLIDAY, vec![(2, ENGLISH, CLASS_STATUS_MOVED_HERE)])
        );
    }

    #[test]
    fn changes_without_a_class_to_act_on_are_ignored() {
        let mut term = term();
        // Nothing is scheduled on Thursday's first period.
        term.changes.push(change(48, date(4, 16), 1, ChangeKind::CHANGE_KIND_CANCELLED));
        term.changes.push(ChangeData {
            to_date: Some(date(4, 17)),
            to_period: Some(1),
            ..change(49, date(4, 16), 2, ChangeKind::CHANGE_KIND_MOVED)
        });
        assert!(resolve_day(&term, date(4, 16)).classes.is_empty());
        assert!(resolve_day(&term, date(4, 17)).classes.is_empty());
        assert!(grid_class(&term, date(4, 16), 1).is_none());
        assert_eq!(grid_class(&term, date(4, 13), 2).map(|class| class.subject_id), Some(ENGLISH));
        assert!(grid_class(&term, date(8, 3), 1).is_none(), "outside the term");
        // An extra class into a period the layout does not have is dropped.
        term.changes
            .push(ChangeData { subject_id: Some(ART), ..change(50, date(4, 16), 9, ChangeKind::CHANGE_KIND_EXTRA) });
        assert!(resolve_day(&term, date(4, 16)).classes.is_empty());
    }

    #[test]
    fn the_current_term_wins_where_terms_overlap() {
        let info = |n: u128, start: NaiveDate, end: NaiveDate, current: bool| TermInfo {
            id: id(n),
            start_date: start,
            end_date: end,
            is_current: current,
            ..term().info
        };
        let first = info(1, date(4, 1), date(9, 30), false);
        let second = info(2, date(9, 1), date(12, 24), false);
        let terms = [first.clone(), second.clone()];
        assert_eq!(term_for(&terms, date(5, 1)).map(|t| t.id), Some(id(1)));
        assert_eq!(term_for(&terms, date(9, 15)).map(|t| t.id), Some(id(2)), "the later term wins the overlap");
        assert_eq!(term_for(&terms, date(3, 1)).map(|t| t.id), None);
        let terms = [TermInfo { is_current: true, ..first }, second];
        assert_eq!(term_for(&terms, date(9, 15)).map(|t| t.id), Some(id(1)), "the current term wins the overlap");
        assert_eq!(out_of_term(date(3, 1)).kind, DayKind::DAY_KIND_OUT_OF_TERM);
        assert_eq!(dates(date(4, 29), date(5, 2)).count(), 4);
        assert_eq!(dates(date(5, 2), date(4, 29)).count(), 0);
    }
}
