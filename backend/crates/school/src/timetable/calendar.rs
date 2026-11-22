//! Someone's timetable as one viewer may see it, loaded for a range of
//! dates: the basis of `GetDay`, `ListDays`, `GetNow`, friends' timetables,
//! free periods, the profile card and the class reminder.
use std::collections::HashMap;

use chrono::{Duration, NaiveDate};
use chrono_tz::Tz;
use tokuly_core::prelude::*;
use tokuly_proto::school::{ClassNow, ScheduledClass, TimetableDay};

use super::{
    grid,
    resolve::{self, ResolvedClass, ResolvedDay, TermData},
    terms,
};
use crate::{
    local,
    subjects::{self, SubjectRow},
};

/// How far ahead `GetNow` looks for the next class.
pub(crate) const LOOKAHEAD_DAYS: i64 = 14;

pub(crate) struct Calendar {
    pub owner: Uuid,
    /// The owner's time zone: their dates and class times are in it.
    pub tz: Tz,
    /// The viewer is the owner (memos are shown).
    pub own: bool,
    /// The visible terms that touch the loaded range.
    terms: Vec<TermData>,
    subjects: HashMap<Uuid, SubjectRow>,
}

impl Calendar {
    /// Loads `owner`'s timetable between two dates as `viewer` may see it.
    /// `None` when nothing of the owner's is shared with the viewer.
    pub async fn load(
        core: &Core,
        owner: Uuid,
        viewer: Uuid,
        start: NaiveDate,
        end: NaiveDate,
    ) -> AppResult<Option<Self>> {
        let visible = terms::visible_to(core, owner, viewer).await?;
        if visible.is_empty() && owner != viewer {
            return Ok(None);
        }
        let touching: Vec<terms::TermRow> =
            visible.into_iter().filter(|term| term.start_date <= end && term.end_date >= start).collect();
        let terms = grid::load(&mut *core.db.acquire().await?, &touching).await?;
        Ok(Some(Self {
            owner,
            tz: local::time_zone(core, owner).await?,
            own: owner == viewer,
            terms,
            subjects: if touching.is_empty() { HashMap::new() } else { subjects::map_of(core, owner).await? },
        }))
    }

    /// Loads what `GetNow` needs at `at`: today and the two weeks after it.
    pub async fn load_around(core: &Core, owner: Uuid, viewer: Uuid, at: Time) -> AppResult<Option<Self>> {
        // The owner's "today" is not known before their time zone is; a day
        // of slack on either side covers every zone.
        let today = at.date_naive();
        Self::load(core, owner, viewer, today - Duration::days(1), today + Duration::days(LOOKAHEAD_DAYS + 1)).await
    }

    pub fn today(&self, at: Time) -> NaiveDate {
        local::local_date(at, self.tz)
    }

    fn term_for(&self, date: NaiveDate) -> Option<&TermData> {
        let info = resolve::term_for(self.terms.iter().map(|term| &term.info), date)?;
        self.terms.iter().find(|term| term.info.id == info.id)
    }

    /// The term that decides `date` and the day it makes of it.
    pub fn resolve(&self, date: NaiveDate) -> (Option<&TermData>, ResolvedDay) {
        match self.term_for(date) {
            Some(term) => (Some(term), resolve::resolve_day(term, date)),
            None => (None, resolve::out_of_term(date)),
        }
    }

    pub fn class_proto(&self, class: &ResolvedClass) -> ScheduledClass {
        let subject = self.subjects.get(&class.subject_id);
        // The class's own teacher, room and colour, else the subject's.
        let or_subject = |own: &str, of_subject: fn(&SubjectRow) -> &String| -> String {
            if own.is_empty() { subject.map(|s| of_subject(s).clone()).unwrap_or_default() } else { own.to_owned() }
        };
        ScheduledClass {
            date: local::fmt_date(class.date),
            period: class.period,
            starts_at: ts(local::at_minute(class.date, class.start_minute, self.tz)),
            ends_at: ts(local::at_minute(class.date, class.end_minute, self.tz)),
            start_minute: class.start_minute,
            end_minute: class.end_minute,
            status: class.status.into(),
            subject: subject.map(SubjectRow::to_proto).into(),
            teacher: or_subject(&class.teacher, |s| &s.teacher),
            room: or_subject(&class.room, |s| &s.room),
            color: or_subject(&class.color, |s| &s.color),
            memo: if self.own { class.memo.clone() } else { String::new() },
            entry_id: class.entry_id.map(|id| id.to_string()).unwrap_or_default(),
            change_id: class.change_id.map(|id| id.to_string()).unwrap_or_default(),
            other_date: class.other.map(|(date, _)| local::fmt_date(date)).unwrap_or_default(),
            other_period: class.other.map(|(_, period)| period).unwrap_or(0),
            original_subject: class
                .original_subject_id
                .and_then(|id| self.subjects.get(&id))
                .map(SubjectRow::to_proto)
                .into(),
            ..Default::default()
        }
    }

    pub fn day_proto(&self, date: NaiveDate) -> TimetableDay {
        let (term, day) = self.resolve(date);
        TimetableDay {
            date: local::fmt_date(date),
            weekday: local::weekday_of(date).into(),
            kind: day.kind.into(),
            term_id: term.map(|term| term.info.id.to_string()).unwrap_or_default(),
            term_name: term.map(|term| term.info.name.clone()).unwrap_or_default(),
            label: day.label.clone(),
            week_pattern: (day.week_pattern as i32).into(),
            follows_weekday: day.follows_weekday.into(),
            classes: day.classes.iter().map(|class| self.class_proto(class)).collect(),
            periods: term.map(|term| term.info.layout.periods_proto()).unwrap_or_default(),
            ..Default::default()
        }
    }

    fn starts_at(&self, class: &ResolvedClass) -> Time {
        local::at_minute(class.date, class.start_minute, self.tz)
    }

    fn ends_at(&self, class: &ResolvedClass) -> Time {
        local::at_minute(class.date, class.end_minute, self.tz)
    }

    /// The classes of a date that take place, in order of time.
    pub fn classes_on(&self, date: NaiveDate) -> Vec<ResolvedClass> {
        let mut classes: Vec<ResolvedClass> =
            self.resolve(date).1.classes.into_iter().filter(ResolvedClass::takes_place).collect();
        classes.sort_by_key(|class| (class.start_minute, class.period));
        classes
    }

    /// The first class of a date that takes place, with its start.
    pub fn first_class(&self, date: NaiveDate) -> Option<(ResolvedClass, Time)> {
        let first = self.classes_on(date).into_iter().next()?;
        let starts_at = self.starts_at(&first);
        Some((first, starts_at))
    }

    /// The class in progress at `at` and the next one, looking up to
    /// `days_ahead` days ahead for the latter (0 = today only).
    pub fn now(&self, at: Time, days_ahead: i64) -> (Option<ResolvedClass>, Option<ResolvedClass>) {
        let today = self.today(at);
        let todays = self.classes_on(today);
        let current = todays.iter().find(|class| self.starts_at(class) <= at && at < self.ends_at(class)).cloned();
        let mut next = todays.into_iter().find(|class| self.starts_at(class) > at);
        let mut ahead = 1;
        while next.is_none() && ahead <= days_ahead {
            next = self.classes_on(today + Duration::days(ahead)).into_iter().next();
            ahead += 1;
        }
        (current, next)
    }

    pub fn now_proto(&self, at: Time) -> ClassNow {
        let (current, next) = self.now(at, LOOKAHEAD_DAYS);
        ClassNow {
            current: current.map(|class| self.class_proto(&class)).into(),
            next: next.map(|class| self.class_proto(&class)).into(),
            server_time: ts(at),
            ..Default::default()
        }
    }

    /// The owner's subject behind an ID of this calendar.
    pub fn subject(&self, id: Uuid) -> Option<&SubjectRow> {
        self.subjects.get(&id)
    }
}
