//! `school.class_soon`: a reminder a few minutes before the first class of
//! the day, for the people who asked for it.
//!
//! Each of them has one "look at me next" time (`school_settings.class_soon_due_at`):
//! the moment the reminder for their next first class is due. The sweep
//! takes the rows that are due, sends what is to be sent, and sets the time
//! for the following day with classes. Anything that changes the timetable
//! or the setting recomputes it ([`School::reschedule_class_soon`]).
use chrono::{Duration, NaiveDate};
use tokuly_core::{
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};

use super::{
    calendar::{Calendar, LOOKAHEAD_DAYS},
    resolve::ResolvedClass,
};
use crate::School;

pub(crate) const KIND_CLASS_SOON: &str = "school.class_soon";

/// When the next reminder is due: `lead` before the first class that has
/// not begun, on a day that has not had its reminder. With no class in
/// sight, a week from now, to look again.
fn next_due(calendar: &Calendar, lead: Duration, sent_on: Option<NaiveDate>, at: Time) -> Time {
    let today = calendar.today(at);
    (0..=LOOKAHEAD_DAYS)
        .map(|offset| today + Duration::days(offset))
        .filter(|date| Some(*date) != sent_on)
        .filter_map(|date| calendar.first_class(date))
        .find(|(_, starts_at)| *starts_at > at)
        .map(|(_, starts_at)| starts_at - lead)
        .unwrap_or(at + Duration::days(7))
}

fn clock(minute: i32) -> String {
    format!("{}:{:02}", minute / 60, minute % 60)
}

fn notification(user_id: Uuid, calendar: &Calendar, class: &ResolvedClass) -> NotificationRequest {
    let subject = calendar.subject(class.subject_id);
    let name = subject.map(|s| s.name.as_str()).unwrap_or("授業");
    let room =
        if class.room.is_empty() { subject.map(|s| s.room.as_str()).unwrap_or_default() } else { class.room.as_str() };
    let starts = clock(class.start_minute);
    let (room_ja, room_en) =
        if room.is_empty() { (String::new(), String::new()) } else { (format!("（{room}）"), format!(" in {room}")) };
    NotificationRequest::new(
        user_id,
        KIND_CLASS_SOON,
        Localized::new("もうすぐ授業!", "Class starts soon"),
        Localized::new(
            format!("{}限 {name}{room_ja} {starts}〜", class.period),
            format!("Period {}: {name} at {starts}{room_en}", class.period),
        ),
        "/school/timetable",
    )
    .collapse(KIND_CLASS_SOON)
    .data(serde_json::json!({
        "date": class.date.to_string(),
        "period": class.period,
        "subject": name,
        "room": room,
        "start_minute": class.start_minute,
    }))
}

impl School {
    /// Recomputes when the reminder worker looks at this user next. Called
    /// after every change to their timetable or to the setting.
    pub(crate) async fn reschedule_class_soon(&self, user_id: Uuid, at: Time) -> AppResult<()> {
        let core = &self.core;
        let row: Option<(i32, Option<NaiveDate>)> =
            sqlx::query_as("SELECT class_soon_minutes, class_soon_sent_on FROM school_settings WHERE user_id = $1")
                .bind(user_id)
                .fetch_optional(&core.db)
                .await?;
        let due = match row {
            // No row: the reminder was never switched on.
            None => return Ok(()),
            Some((0, _)) => None,
            Some((minutes, sent_on)) => {
                let calendar = Calendar::load_around(core, user_id, user_id, at)
                    .await?
                    .expect("one's own timetable is always visible");
                Some(next_due(&calendar, Duration::minutes(minutes as i64), sent_on, at))
            }
        };
        sqlx::query("UPDATE school_settings SET class_soon_due_at = $2 WHERE user_id = $1")
            .bind(user_id)
            .bind(due)
            .execute(&core.db)
            .await?;
        Ok(())
    }

    /// Sends the class reminders that are due at `at`. Returns how many were
    /// sent. Safe to run in several processes at once.
    pub async fn send_class_reminders(&self, at: Time) -> AppResult<usize> {
        let due: Vec<Uuid> = sqlx::query_scalar(
            "SELECT user_id FROM school_settings WHERE class_soon_due_at <= $1 ORDER BY class_soon_due_at LIMIT 200",
        )
        .bind(at)
        .fetch_all(&self.core.db)
        .await?;
        let mut sent = 0;
        for user_id in due {
            match self.class_reminder(user_id, at).await {
                Ok(true) => sent += 1,
                Ok(false) => {}
                Err(e) => tracing::error!(%user_id, error = %e, "class reminder failed"),
            }
        }
        Ok(sent)
    }

    /// Handles one user whose reminder time has come: sends the reminder if
    /// their first class of today is within the lead time and has not begun,
    /// and schedules the next one either way.
    async fn class_reminder(&self, user_id: Uuid, at: Time) -> AppResult<bool> {
        let core = &self.core;
        let mut tx = core.db.begin().await?;
        let row: Option<(i32, Option<NaiveDate>)> = sqlx::query_as(
            "SELECT class_soon_minutes, class_soon_sent_on FROM school_settings
             WHERE user_id = $1 AND class_soon_due_at <= $2 FOR UPDATE SKIP LOCKED",
        )
        .bind(user_id)
        .bind(at)
        .fetch_optional(&mut *tx)
        .await?;
        // Another worker has it, or it was rescheduled in the meantime.
        let Some((minutes, mut sent_on)) = row else { return Ok(false) };
        if minutes == 0 {
            sqlx::query("UPDATE school_settings SET class_soon_due_at = NULL WHERE user_id = $1")
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            return Ok(false);
        }
        let lead = Duration::minutes(minutes as i64);
        let calendar =
            Calendar::load_around(core, user_id, user_id, at).await?.expect("one's own timetable is always visible");
        let today = calendar.today(at);
        let mut sent = false;
        if sent_on != Some(today)
            && let Some((class, starts_at)) = calendar.first_class(today)
            && starts_at - lead <= at
            && at < starts_at
        {
            outbound::notify(&mut *tx, &notification(user_id, &calendar, &class)).await?;
            sent_on = Some(today);
            sent = true;
        }
        sqlx::query("UPDATE school_settings SET class_soon_due_at = $2, class_soon_sent_on = $3 WHERE user_id = $1")
            .bind(user_id)
            .bind(next_due(&calendar, lead, sent_on, at))
            .bind(sent_on)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(sent)
    }
}
