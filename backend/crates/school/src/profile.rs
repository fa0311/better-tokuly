//! What the school domain shows on profiles: "勉強中", the class someone is
//! in (or has next today), and "テスト期間".
//!
//! All three are for friends at most, and each has its own switch: the
//! study visibility setting, the audience of the term, the exam's
//! "show on profile". A stranger's view of a profile gets nothing from here.
use async_trait::async_trait;
use tokuly_core::prelude::*;
use tokuly_identity::ProfileExtension;
use tokuly_proto::{
    common::LocalizedText,
    identity::{Profile, ProfileCard},
    school::{ExamStatus, SessionState, StudyingFriend, TimerPhase},
};
use tokuly_social::graph;

use crate::{
    School,
    exams::{self, ExamRow},
    local, rooms,
    timetable::{ResolvedClass, calendar::Calendar},
};

pub(crate) const CARD_STUDYING: &str = "school.studying";
pub(crate) const CARD_NOW: &str = "school.now";
pub(crate) const CARD_EXAM: &str = "school.exam";

fn text(ja: impl Into<String>, en: impl Into<String>) -> LocalizedText {
    LocalizedText { ja: ja.into(), en: en.into(), ..Default::default() }
}

fn clock(minute: i32) -> String {
    format!("{}:{:02}", minute / 60, minute % 60)
}

fn studying_card(studying: &StudyingFriend) -> ProfileCard {
    let minutes = studying.studied_seconds / 60;
    let resting =
        studying.state == SessionState::SESSION_STATE_PAUSED || studying.phase != TimerPhase::TIMER_PHASE_WORK;
    let subject = studying.subject.as_option().map(|subject| subject.name.clone()).unwrap_or_default();
    let (ja, en) = if subject.is_empty() {
        (format!("{minutes}分"), format!("{minutes} min"))
    } else {
        (format!("{subject} · {minutes}分"), format!("{subject} · {minutes} min"))
    };
    ProfileCard {
        kind: CARD_STUDYING.into(),
        title: if resting { text("休憩中", "Taking a break") } else { text("勉強中", "Studying") }.into(),
        text: text(ja, en).into(),
        emoji: if resting { "☕" } else { "📖" }.into(),
        path: match studying.room_id.parse::<Uuid>() {
            Ok(room_id) => rooms::path(room_id),
            Err(_) => "/school/study".into(),
        },
        data: [
            ("session_id", studying.session_id.clone()),
            (
                "state",
                if studying.state == SessionState::SESSION_STATE_PAUSED { "paused" } else { "running" }.to_owned(),
            ),
            ("resting", resting.to_string()),
            ("subject", subject),
            ("studied_seconds", studying.studied_seconds.to_string()),
            ("started_at", studying.started_at.as_option().map(|at| from_ts(at).to_rfc3339()).unwrap_or_default()),
            ("room_id", studying.room_id.clone()),
            ("room_title", studying.room_title.clone()),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect(),
        order: 20,
        ..Default::default()
    }
}

fn class_card(calendar: &Calendar, class: &ResolvedClass, current: bool) -> ProfileCard {
    let proto = calendar.class_proto(class);
    let subject = proto.subject.as_option().map(|subject| subject.name.clone()).unwrap_or_default();
    let (title, body) = if current {
        let until = clock(class.end_minute);
        (
            text("授業中", "In class"),
            text(
                format!("{}限 {subject}（〜{until}）", class.period),
                format!("Period {}: {subject} (until {until})", class.period),
            ),
        )
    } else {
        let from = clock(class.start_minute);
        (
            text("次の授業", "Next class"),
            text(
                format!("{}限 {subject} {from}〜", class.period),
                format!("Period {}: {subject} at {from}", class.period),
            ),
        )
    };
    ProfileCard {
        kind: CARD_NOW.into(),
        title: title.into(),
        text: body.into(),
        emoji: "🏫".into(),
        path: if calendar.own { "/school/timetable".into() } else { format!("/school/timetable/{}", calendar.owner) },
        data: [
            ("state", if current { "current" } else { "next" }.to_owned()),
            ("period", class.period.to_string()),
            ("subject", subject),
            ("room", proto.room.clone()),
            ("starts_at", proto.starts_at.as_option().map(|at| from_ts(at).to_rfc3339()).unwrap_or_default()),
            ("ends_at", proto.ends_at.as_option().map(|at| from_ts(at).to_rfc3339()).unwrap_or_default()),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect(),
        order: 30,
        ..Default::default()
    }
}

fn exam_card(exam: &ExamRow, today: chrono::NaiveDate, own: bool) -> ProfileCard {
    let (status, days_until_start, days_left) = exams::countdown(exam.start_date, exam.end_date, today);
    let body = if status == ExamStatus::EXAM_STATUS_ONGOING {
        text(
            format!("{} · テスト期間中（あと{days_left}日）", exam.name),
            format!("{} · on now ({days_left} days left)", exam.name),
        )
    } else {
        text(format!("{} · あと{days_until_start}日", exam.name), format!("{} · in {days_until_start} days", exam.name))
    };
    ProfileCard {
        kind: CARD_EXAM.into(),
        title: text("テスト期間", "Exams").into(),
        text: body.into(),
        emoji: "📝".into(),
        // The exam itself (subjects, results) is its owner's alone.
        path: if own { format!("/school/exams/{}", exam.id) } else { String::new() },
        data: [
            ("status", if status == ExamStatus::EXAM_STATUS_ONGOING { "ongoing" } else { "upcoming" }.to_owned()),
            ("name", exam.name.clone()),
            ("days_until_start", days_until_start.to_string()),
            ("days_left", days_left.to_string()),
            ("start_date", local::fmt_date(exam.start_date)),
            ("end_date", local::fmt_date(exam.end_date)),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect(),
        order: 40,
        ..Default::default()
    }
}

#[async_trait]
impl ProfileExtension for School {
    async fn enrich_profile(&self, viewer: Option<Uuid>, owner: Uuid, profile: &mut Profile) -> AppResult<()> {
        let core = &self.core;
        let Some(viewer) = viewer else { return Ok(()) };
        let own = viewer == owner;
        if !own && !graph::are_friends(core, owner, viewer).await? {
            return Ok(());
        }
        let at = self.now();

        // Shown to friends only when the owner shows their studying (the
        // owner sees on their own profile what their friends see).
        if let Some(studying) = self.studying_now(owner, Some(viewer), at).await? {
            profile.cards.push(studying_card(&studying));
        }

        // The class in progress, else the next one today — for viewers in
        // the audience of the term.
        if let Some(calendar) = Calendar::load_around(core, owner, viewer, at).await? {
            let (current, next) = calendar.now(at, 0);
            if let Some(class) = current.as_ref().or(next.as_ref()) {
                profile.cards.push(class_card(&calendar, class, current.is_some()));
            }
        }

        let today = local::local_date(at, local::time_zone(core, owner).await?);
        if let Some(exam) = self.profile_exam(owner, today).await? {
            profile.cards.push(exam_card(&exam, today, own));
        }
        Ok(())
    }
}
