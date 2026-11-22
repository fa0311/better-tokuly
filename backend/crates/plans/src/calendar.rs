//! The calendar: one month of everything dated for a user — plans, poll
//! deadlines, countdowns, anniversary milestones and birthdays — in one list.
use buffa::Enumeration as _;
use chrono::{Datelike as _, Months, NaiveDate};
use chrono_tz::Tz;
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::{outbound::Localized, prelude::*};
use tokuly_proto::{common::LocalizedText, plans::*};

use crate::{
    Plans, anniversaries::anniversary_path, copy, countdowns::countdown_path, dates, input, milestones,
    people::name_of, plan::plan_path,
};

/// One line of the calendar before names are filled in.
struct Entry {
    kind: CalendarEntryKind,
    date: NaiveDate,
    /// Unset for all-day entries.
    start_at: Option<Time>,
    title: String,
    emoji: String,
    ref_id: Option<Uuid>,
    /// Whose birthday, the partner of a counter, or the owner of a shared countdown.
    user: Option<Uuid>,
    /// Builds the server-written label from that user's name.
    label: Option<Box<dyn Fn(&str) -> Localized + Send>>,
    path: String,
    rsvp: Rsvp,
}

impl Entry {
    fn new(kind: CalendarEntryKind, date: NaiveDate) -> Self {
        Self {
            kind,
            date,
            start_at: None,
            title: String::new(),
            emoji: String::new(),
            ref_id: None,
            user: None,
            label: None,
            path: String::new(),
            rsvp: Rsvp::RSVP_UNSPECIFIED,
        }
    }
}

fn constant(label: (&'static str, &'static str)) -> Option<Box<dyn Fn(&str) -> Localized + Send>> {
    Some(Box::new(move |_| Localized::new(label.0, label.1)))
}

/// The first and last day of a month written `YYYY-MM`.
fn month_range(month: &str) -> AppResult<(NaiveDate, NaiveDate)> {
    let first = (month.len() == 7).then(|| dates::parse_date(&format!("{month}-01"))).flatten();
    let last = first.and_then(|first| first.checked_add_months(Months::new(1))).and_then(|next| next.pred_opt());
    first.zip(last).ok_or_else(|| AppError::invalid("MONTH_INVALID", "the month is written YYYY-MM"))
}

impl Plans {
    /// Everything dated for `user_id` in `month`, sorted by day, all-day
    /// entries before timed ones.
    pub(crate) async fn calendar(&self, user_id: Uuid, month: &str, time_zone: &str) -> AppResult<Vec<CalendarEntry>> {
        let core = &self.core;
        let (first, last) = month_range(month)?;
        let zone: Tz = input::zone(time_zone, dates::zone_of(core, user_id).await?)?;
        let (from, until) = (dates::day_start(first, zone), dates::day_end(last, zone));
        let in_month = |date: &NaiveDate| (first..=last).contains(date);
        let mut entries: Vec<Entry> = Vec::new();

        // Plans the caller has not declined: on their start day.
        let plans: Vec<(Uuid, String, String, Option<Time>, Option<NaiveDate>, i16)> = sqlx::query_as(
            "SELECT p.id, p.title, p.emoji, p.start_at, p.start_date, me.rsvp
             FROM plans p JOIN plan_participants me ON me.plan_id = p.id AND me.user_id = $1
             WHERE p.removed_at IS NULL AND p.status = 1 AND me.rsvp <> 4
               AND ((p.start_kind = 1 AND p.start_at >= $2 AND p.start_at < $3) OR (p.start_kind = 2 AND p.start_date BETWEEN $4 AND $5))",
        )
        .bind(user_id)
        .bind(from)
        .bind(until)
        .bind(first)
        .bind(last)
        .fetch_all(&core.db)
        .await?;
        for (id, title, emoji, start_at, start_date, rsvp) in plans {
            let Some(date) = start_at.map(|at| dates::day_in(zone, at)).or(start_date) else { continue };
            entries.push(Entry {
                start_at,
                title,
                emoji,
                ref_id: Some(id),
                path: plan_path(id),
                rsvp: Rsvp::from_i32(rsvp as i32).unwrap_or(Rsvp::RSVP_UNSPECIFIED),
                ..Entry::new(CalendarEntryKind::CALENDAR_ENTRY_KIND_PLAN, date)
            });
        }

        // Deadlines of the open polls the caller is in.
        let polls: Vec<(Uuid, String, String, Time)> = sqlx::query_as(
            "SELECT q.id, q.short_id, q.title, q.deadline
             FROM date_polls q JOIN date_poll_participants me ON me.poll_id = q.id AND me.user_id = $1
             WHERE q.status = 1 AND q.deadline >= $2 AND q.deadline < $3",
        )
        .bind(user_id)
        .bind(from)
        .bind(until)
        .fetch_all(&core.db)
        .await?;
        for (id, short_id, title, deadline) in polls {
            entries.push(Entry {
                start_at: Some(deadline),
                title,
                ref_id: Some(id),
                label: constant(copy::POLL_DEADLINE_LABEL),
                path: format!("/plans/polls/{short_id}"),
                ..Entry::new(CalendarEntryKind::CALENDAR_ENTRY_KIND_POLL_DEADLINE, dates::day_in(zone, deadline))
            });
        }

        // Countdowns: the caller's own and the ones shared with them.
        for countdown in self.visible_countdowns(user_id).await? {
            let Some(date) = countdown.date_in(first.year()).filter(in_month) else { continue };
            entries.push(Entry {
                title: countdown.title.clone(),
                emoji: countdown.emoji.clone(),
                ref_id: Some(countdown.id),
                user: (countdown.user_id != user_id).then_some(countdown.user_id),
                path: countdown_path(countdown.id),
                ..Entry::new(CalendarEntryKind::CALENDAR_ENTRY_KIND_COUNTDOWN, date)
            });
        }

        // Milestones of the caller's counters, one per day.
        for counter in self.anniversaries_of(user_id).await?.into_iter().filter(|counter| counter.is_active()) {
            let mut found = milestones::between(counter.start_date, counter.count_start_day, first, last);
            found.dedup_by_key(|milestone| milestone.date);
            for milestone in found {
                entries.push(Entry {
                    title: counter.title.clone(),
                    emoji: counter.emoji.clone(),
                    ref_id: Some(counter.id),
                    user: Some(counter.partner_of(user_id)),
                    label: Some(Box::new(move |partner| copy::milestone_label(partner, milestone.mark))),
                    path: anniversary_path(counter.id),
                    ..Entry::new(CalendarEntryKind::CALENDAR_ENTRY_KIND_ANNIVERSARY, milestone.date)
                });
            }
        }

        // Birthdays: friends who share theirs, and the caller's own.
        for birthday in self.shared_birthdays(user_id).await? {
            let date = dates::yearly_in(birthday.month, birthday.day, first.year());
            if in_month(&date) {
                entries.push(Entry {
                    user: Some(birthday.user_id),
                    label: Some(Box::new(copy::birthday_label)),
                    ..Entry::new(CalendarEntryKind::CALENDAR_ENTRY_KIND_BIRTHDAY, date)
                });
            }
        }
        if let Some(birthday) = self.own_birthday(user_id).await? {
            let date = dates::yearly_in(birthday.month, birthday.day, first.year());
            if in_month(&date) {
                entries.push(Entry {
                    user: Some(user_id),
                    label: constant(copy::OWN_BIRTHDAY_LABEL),
                    ..Entry::new(CalendarEntryKind::CALENDAR_ENTRY_KIND_BIRTHDAY, date)
                });
            }
        }

        entries.sort_by_key(|entry| {
            (entry.date, entry.start_at.is_some(), entry.start_at, entry.kind as i32, entry.ref_id, entry.user)
        });
        let named: Vec<Uuid> = entries.iter().filter_map(|entry| entry.user).collect();
        let summaries = tokuly_identity::summaries(core, &named).await?;
        Ok(entries
            .into_iter()
            .map(|entry| {
                let user = entry.user.map(|id| tokuly_identity::users::pick(&summaries, id));
                let label = entry.label.as_ref().map(|label| label(&user.as_ref().map(name_of).unwrap_or_default()));
                CalendarEntry {
                    kind: entry.kind.into(),
                    date: dates::format_date(entry.date),
                    start_at: opt_ts(entry.start_at),
                    title: entry.title,
                    label: label.map(|label| LocalizedText { ja: label.ja, en: label.en, ..Default::default() }).into(),
                    emoji: entry.emoji,
                    ref_id: entry.ref_id.map(|id| id.to_string()).unwrap_or_default(),
                    user: user.into(),
                    path: entry.path,
                    rsvp: entry.rsvp.into(),
                    ..Default::default()
                }
            })
            .collect())
    }
}

impl CalendarService for Plans {
    async fn get_calendar(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetCalendarRequest>,
    ) -> ServiceResult<GetCalendarResponse> {
        handle(async {
            let entries = self.calendar(ctx.user_id()?, request.month, request.time_zone).await?;
            ok(GetCalendarResponse { entries, ..Default::default() })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_month_runs_from_its_first_to_its_last_day() {
        let day = |text: &str| dates::parse_date(text).unwrap();
        assert_eq!(month_range("2026-11").unwrap(), (day("2026-11-01"), day("2026-11-30")));
        assert_eq!(month_range("2026-12").unwrap(), (day("2026-12-01"), day("2026-12-31")));
        assert_eq!(month_range("2028-02").unwrap(), (day("2028-02-01"), day("2028-02-29")));
        for bad in ["", "2026", "2026-13", "2026-1", "2026-11-01", "11-2026", "20261-1"] {
            assert_eq!(month_range(bad).unwrap_err().reason, "MONTH_INVALID", "{bad}");
        }
    }
}
