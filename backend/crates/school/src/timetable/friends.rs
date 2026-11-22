//! Timetables between friends: who shares theirs, reading a friend's,
//! copying a classmate's, and finding free periods in common.
use std::collections::{HashMap, HashSet};

use chrono::{Duration, NaiveDate};
use tokuly_core::prelude::*;
use tokuly_proto::{
    common::PageInfo,
    school::{
        CopyFriendTimetableRequest, CopyFriendTimetableResponse, DayKind, FindCommonFreePeriodsRequest,
        FindCommonFreePeriodsResponse, FreePeriod, FriendTimetable, GetFriendTimetableResponse,
    },
};
use tokuly_social::graph;

use super::{
    calendar::Calendar,
    grid,
    resolve::{self, EVERY},
    terms::{self, AUDIENCE_CHOSEN, AUDIENCE_FRIENDS, TermRow},
};
use crate::{School, local, subjects, text};

const MAX_COMPARED: usize = 50;
const MAX_FREE_DAYS: i32 = 7;

fn not_shared() -> AppError {
    AppError::not_found("TIMETABLE_NOT_SHARED", "this timetable is not shared with you")
}

impl School {
    /// The current terms of `friends` that are shared with `viewer`, by
    /// owner, after `after`.
    async fn shared_current_terms(
        &self,
        viewer: Uuid,
        friends: &[Uuid],
        after: Option<Uuid>,
        limit: i64,
    ) -> AppResult<Vec<TermRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM school_terms t
             WHERE t.user_id = ANY($1) AND t.is_current
               AND (t.audience = {AUDIENCE_FRIENDS}
                    OR (t.audience = {AUDIENCE_CHOSEN} AND EXISTS (SELECT 1 FROM school_term_shares s WHERE s.term_id = t.id AND s.user_id = $2)))
               AND ($3::uuid IS NULL OR t.user_id > $3)
             ORDER BY t.user_id LIMIT $4",
            terms::COLUMNS
        )))
        .bind(friends)
        .bind(viewer)
        .bind(after)
        .bind(limit)
        .fetch_all(&self.core.db)
        .await?)
    }

    pub(crate) async fn list_friend_timetables(
        &self,
        viewer: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<FriendTimetable>, PageInfo)> {
        let core = &self.core;
        let at = self.now();
        let friends = graph::friend_ids(core, viewer).await?;
        let mut rows = self.shared_current_terms(viewer, &friends, page.decode()?, page.fetch()).await?;
        let info = page.finish(&mut rows, |term| term.user_id);
        let owners: Vec<Uuid> = rows.iter().map(|term| term.user_id).collect();
        let users = tokuly_identity::summaries(core, &owners).await?;
        let mut timetables = Vec::with_capacity(rows.len());
        for term in &rows {
            let Some(user) = users.get(&term.user_id).filter(|user| !user.deleted) else { continue };
            // The friendship may have ended since the friend list was cached.
            let Some(calendar) = Calendar::load_around(core, term.user_id, viewer, at).await? else { continue };
            timetables.push(FriendTimetable {
                user: user.clone().into(),
                term: term.to_proto().into(),
                now: calendar.now_proto(at).into(),
                ..Default::default()
            });
        }
        Ok((timetables, info))
    }

    /// The term of `owner` that `viewer` asked for (their current one by
    /// default) and the other terms shared with the viewer.
    async fn shared_term(
        &self,
        owner: Uuid,
        viewer: Uuid,
        term_id: Option<Uuid>,
    ) -> AppResult<(TermRow, Vec<TermRow>)> {
        let mut visible = terms::visible_to(&self.core, owner, viewer).await?;
        let index = match term_id {
            Some(id) => visible.iter().position(|term| term.id == id),
            // The current term, else the newest one that is shared.
            None => visible.iter().position(|term| term.is_current).or(if visible.is_empty() { None } else { Some(0) }),
        };
        let term = visible.remove(index.ok_or_else(not_shared)?);
        Ok((term, visible))
    }

    pub(crate) async fn friend_timetable(
        &self,
        viewer: Uuid,
        owner: Uuid,
        term_id: Option<Uuid>,
    ) -> AppResult<GetFriendTimetableResponse> {
        let core = &self.core;
        let (term, others) = self.shared_term(owner, viewer, term_id).await?;
        let data = grid::load_one(&mut *core.db.acquire().await?, &term).await?;
        let subjects = subjects::map_of(core, owner).await?;
        let own = owner == viewer;
        Ok(GetFriendTimetableResponse {
            term: if own { self.term_for_owner(&term).await? } else { term.to_proto() }.into(),
            entries: data.entries.iter().map(|entry| grid::entry_proto(term.id, entry, &subjects, own)).collect(),
            other_terms: others.iter().map(TermRow::to_proto).collect(),
            ..Default::default()
        })
    }

    pub(crate) async fn copy_friend_timetable(
        &self,
        viewer: Uuid,
        request: &CopyFriendTimetableRequest,
    ) -> AppResult<CopyFriendTimetableResponse> {
        let core = &self.core;
        let owner = parse_id(&request.user_id)?;
        let (source, _) = self.shared_term(owner, viewer, parse_opt_id(&request.term_id)?).await?;
        let source_data = grid::load_one(&mut *core.db.acquire().await?, &source).await?;
        let source_subjects = subjects::map_of(core, owner).await?;
        let target_id = parse_opt_id(&request.target_term_id)?;
        let name = match request.name.trim() {
            "" => None,
            name => Some(text::name(name, text::MAX_NAME, "TERM_NAME_INVALID", "term name")?),
        };

        let mut tx = core.db.begin().await?;
        crate::lock_user(&mut tx, "terms", viewer).await?;
        crate::lock_user(&mut tx, "subjects", viewer).await?;
        let (target, is_new) = match target_id {
            Some(id) => (terms::own(&mut tx, viewer, id, true).await?, false),
            None => {
                let id = terms::insert_copy(&mut tx, viewer, &source, name).await?;
                (terms::own(&mut tx, viewer, id, false).await?, true)
            }
        };
        let target_info = target.info();

        // The caller's subjects by name; missing ones are created as the
        // classmate has them.
        let mine: Vec<subjects::SubjectRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM school_subjects WHERE user_id = $1",
            subjects::COLUMNS
        )))
        .bind(viewer)
        .fetch_all(&mut *tx)
        .await?;
        let mut subject_count = mine.len() as i64;
        let mut by_name: HashMap<String, Uuid> =
            mine.into_iter().map(|subject| (subject.name.to_lowercase(), subject.id)).collect();
        let mut created_subjects = 0;

        let occupied: HashSet<(i16, i16)> =
            sqlx::query_as("SELECT DISTINCT weekday, period FROM school_entries WHERE term_id = $1")
                .bind(target.id)
                .fetch_all(&mut *tx)
                .await?
                .into_iter()
                .collect();
        let mut cleared: HashSet<(i16, i16)> = HashSet::new();
        let (mut copied, mut skipped) = (0, 0);
        for entry in &source_data.entries {
            let slot = (entry.weekday, entry.period);
            let fits = target_info.layout.has_weekday(entry.weekday as i32)
                && target_info.layout.period(entry.period as i32).is_some()
                && (entry.week_pattern == EVERY || target_info.alternating_weeks);
            let Some(subject) = source_subjects.get(&entry.subject_id).filter(|_| fits) else {
                skipped += 1;
                continue;
            };
            if occupied.contains(&slot) {
                if !request.overwrite {
                    skipped += 1;
                    continue;
                }
                // The classmate's slot replaces the caller's, whatever weeks it held.
                if cleared.insert(slot) {
                    sqlx::query("DELETE FROM school_entries WHERE term_id = $1 AND weekday = $2 AND period = $3")
                        .bind(target.id)
                        .bind(entry.weekday)
                        .bind(entry.period)
                        .execute(&mut *tx)
                        .await?;
                }
            }
            let subject_id = match by_name.get(&subject.name.to_lowercase()) {
                Some(id) => *id,
                None => {
                    ensure!(
                        subject_count < subjects::MAX_SUBJECTS,
                        precondition,
                        "SUBJECT_LIMIT_REACHED",
                        "an account has at most {} subjects",
                        subjects::MAX_SUBJECTS
                    );
                    let row = subjects::insert(
                        &mut tx,
                        viewer,
                        &subject.name,
                        &subject.short_name,
                        &subject.color,
                        &subject.emoji,
                        &subject.teacher,
                        &subject.room,
                    )
                    .await?;
                    subject_count += 1;
                    created_subjects += 1;
                    by_name.insert(subject.name.to_lowercase(), row.id);
                    row.id
                }
            };
            sqlx::query(
                "INSERT INTO school_entries (term_id, user_id, weekday, period, week_pattern, subject_id, teacher, room, color)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(target.id)
            .bind(viewer)
            .bind(entry.weekday)
            .bind(entry.period)
            .bind(entry.week_pattern)
            .bind(subject_id)
            .bind(&entry.teacher)
            .bind(&entry.room)
            .bind(&entry.color)
            .execute(&mut *tx)
            .await?;
            copied += 1;
        }
        if is_new {
            // The school's calendar comes along: holidays, class days, pinned weeks.
            for day_override in &source_data.overrides {
                sqlx::query(
                    "INSERT INTO school_day_overrides (term_id, user_id, start_date, end_date, kind, name, follows_weekday, week_pattern)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                )
                .bind(target.id)
                .bind(viewer)
                .bind(day_override.start_date)
                .bind(day_override.end_date)
                .bind(day_override.kind)
                .bind(&day_override.name)
                .bind(day_override.follows_weekday)
                .bind(day_override.week_pattern)
                .execute(&mut *tx)
                .await?;
            }
        }
        let term = terms::own(&mut tx, viewer, target.id, false).await?;
        let data = grid::load_one(&mut tx, &term).await?;
        tx.commit().await?;

        let my_subjects = subjects::map_of(core, viewer).await?;
        Ok(CopyFriendTimetableResponse {
            term: self.term_for_owner(&term).await?.into(),
            entries: data.entries.iter().map(|entry| grid::entry_proto(term.id, entry, &my_subjects, true)).collect(),
            copied_entry_count: copied,
            skipped_entry_count: skipped,
            created_subject_count: created_subjects,
            ..Default::default()
        })
    }

    pub(crate) async fn common_free_periods(
        &self,
        viewer: Uuid,
        request: &FindCommonFreePeriodsRequest,
    ) -> AppResult<FindCommonFreePeriodsResponse> {
        let core = &self.core;
        let at = self.now();
        let days = if request.days == 0 { 1 } else { request.days };
        ensure!(
            (1..=MAX_FREE_DAYS).contains(&days),
            invalid,
            "DATE_RANGE_INVALID",
            "free periods are searched over 1 to {MAX_FREE_DAYS} days"
        );
        ensure!(
            request.user_ids.len() <= MAX_COMPARED,
            invalid,
            "TOO_MANY_USERS",
            "compare with at most {MAX_COMPARED} friends at a time"
        );
        let tz = local::time_zone(core, viewer).await?;
        let start = local::parse_opt_date(&request.start_date)?.unwrap_or_else(|| local::local_date(at, tz));
        let end = start + Duration::days(days as i64 - 1);
        let mine =
            Calendar::load(core, viewer, viewer, start, end).await?.expect("one's own timetable is always visible");

        let asked: Vec<Uuid> = if request.user_ids.is_empty() {
            let friends = graph::friend_ids(core, viewer).await?;
            self.shared_current_terms(viewer, &friends, None, MAX_COMPARED as i64)
                .await?
                .into_iter()
                .map(|term| term.user_id)
                .collect()
        } else {
            let mut seen = HashSet::new();
            parse_ids(&request.user_ids)?.into_iter().filter(|id| *id != viewer && seen.insert(*id)).collect()
        };
        let mut theirs = Vec::new();
        let mut not_shared_user_ids = Vec::new();
        for friend in asked {
            // A day of slack: the friend's dates may differ from the caller's.
            match Calendar::load(core, friend, viewer, start - Duration::days(1), end + Duration::days(1)).await? {
                Some(calendar) => theirs.push(calendar),
                None => not_shared_user_ids.push(friend.to_string()),
            }
        }
        let users =
            tokuly_identity::summaries(core, &theirs.iter().map(|calendar| calendar.owner).collect::<Vec<_>>()).await?;

        let mut periods = Vec::new();
        for date in resolve::dates(start, end) {
            let (Some(term), day) = mine.resolve(date) else { continue };
            if day.kind != DayKind::DAY_KIND_SCHOOL_DAY {
                continue;
            }
            let busy: Vec<i32> =
                day.classes.iter().filter(|class| class.takes_place()).map(|class| class.period).collect();
            for period in term.info.layout.periods.iter().filter(|period| !busy.contains(&period.number)) {
                let starts_at = local::at_minute(date, period.start_minute, tz);
                let ends_at = local::at_minute(date, period.end_minute, tz);
                let free_friends = theirs
                    .iter()
                    .filter(|calendar| is_free(calendar, starts_at, ends_at))
                    .filter_map(|calendar| users.get(&calendar.owner).filter(|user| !user.deleted).cloned())
                    .collect();
                periods.push(FreePeriod {
                    date: local::fmt_date(date),
                    period: period.number,
                    starts_at: ts(starts_at),
                    ends_at: ts(ends_at),
                    start_minute: period.start_minute,
                    end_minute: period.end_minute,
                    between_classes: busy.iter().any(|p| *p < period.number) && busy.iter().any(|p| *p > period.number),
                    free_friends,
                    ..Default::default()
                });
            }
        }
        Ok(FindCommonFreePeriodsResponse { periods, not_shared_user_ids, ..Default::default() })
    }
}

/// Whether the calendar's owner has no class between two instants, on days
/// their timetable says something about (out of term, nobody knows).
fn is_free(calendar: &Calendar, from: Time, to: Time) -> bool {
    let first: NaiveDate = local::local_date(from, calendar.tz);
    let last: NaiveDate = local::local_date(to - Duration::milliseconds(1), calendar.tz);
    resolve::dates(first, last).all(|date| {
        let (_, day) = calendar.resolve(date);
        day.kind != DayKind::DAY_KIND_OUT_OF_TERM
            && !day.classes.iter().filter(|class| class.takes_place()).any(|class| {
                local::at_minute(date, class.start_minute, calendar.tz) < to
                    && local::at_minute(date, class.end_minute, calendar.tz) > from
            })
    })
}
