//! Countdowns to a day: an exam, a festival, a concert, a trip.
//!
//! A countdown belongs to one user. It is private, shared with chosen
//! friends, or pinned to the owner's profile (where their friends see it).
//! Days are counted in the owner's time zone, so "あと12日" reads the same
//! for everyone looking at it.
use std::collections::{HashMap, HashSet};

use buffa::Enumeration as _;
use chrono::{Datelike as _, NaiveDate};
use chrono_tz::Tz;
use tokuly_core::{
    outbound::{self, NotificationRequest},
    page::encode_cursor,
    prelude::*,
};
use tokuly_proto::{common::PageInfo, plans::*};
use tokuly_social::graph;

use crate::{Plans, copy, dates, input, people};

const MAX_TITLE: usize = 60;
const MAX_SHARED: usize = 50;
const MAX_COUNTDOWNS: i64 = 100;
/// Countdowns one user can pin to their profile.
pub(crate) const MAX_PINNED: usize = 3;
/// Most countdowns of friends listed next to the caller's own.
const MAX_SHARED_LISTED: i64 = 200;

pub(crate) fn countdown_path(id: Uuid) -> String {
    format!("/plans/countdowns/{id}")
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct CountdownRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub title: String,
    pub emoji: String,
    pub target_date: NaiveDate,
    pub repeats_yearly: bool,
    pub sharing: i16,
    pub shared_ids: Vec<Uuid>,
    pub created_at: Time,
}

macro_rules! countdown_columns {
    () => {
        "c.id, c.user_id, c.title, c.emoji, c.target_date, c.repeats_yearly, c.sharing, c.shared_ids, c.created_at"
    };
}

impl CountdownRow {
    fn sharing(&self) -> CountdownSharing {
        CountdownSharing::from_i32(self.sharing as i32).unwrap_or(CountdownSharing::COUNTDOWN_SHARING_UNSPECIFIED)
    }

    /// The occurrence being counted down to on `today`: the day itself, or
    /// its next anniversary for a countdown that repeats.
    pub(crate) fn next_date(&self, today: NaiveDate) -> NaiveDate {
        if self.repeats_yearly {
            dates::next_yearly(self.target_date.month(), self.target_date.day(), today)
        } else {
            self.target_date
        }
    }

    /// The occurrence falling into `year`, if there is one.
    pub(crate) fn date_in(&self, year: i32) -> Option<NaiveDate> {
        if self.repeats_yearly {
            (year >= self.target_date.year())
                .then(|| dates::yearly_in(self.target_date.month(), self.target_date.day(), year))
        } else {
            (self.target_date.year() == year).then_some(self.target_date)
        }
    }

    /// When the owner is next told "today is the day": the morning of the
    /// next occurrence that still lies ahead of `now`.
    fn notify_due(&self, zone: Tz, now: Time, morning_hour: u32) -> Option<Time> {
        let today = dates::day_in(zone, now);
        let next = dates::local_instant(self.next_date(today), morning_hour, zone);
        if next > now {
            Some(next)
        } else if self.repeats_yearly {
            Some(dates::local_instant(self.next_date(dates::next_day(today)), morning_hour, zone))
        } else {
            None
        }
    }

    pub(crate) fn to_proto(&self, owner: tokuly_proto::common::UserSummary, today: NaiveDate) -> Countdown {
        let next = self.next_date(today);
        Countdown {
            id: self.id.to_string(),
            owner: owner.into(),
            title: self.title.clone(),
            emoji: self.emoji.clone(),
            date: dates::format_date(self.target_date),
            repeats_yearly: self.repeats_yearly,
            next_date: dates::format_date(next),
            days_left: dates::days_until(next, today),
            created_at: ts(self.created_at),
            ..Default::default()
        }
    }
}

fn countdown_title(input: &str) -> AppResult<String> {
    input::required_line(input, MAX_TITLE, "TITLE_REQUIRED", "TITLE_TOO_LONG")
}

fn not_found() -> AppError {
    AppError::not_found("COUNTDOWN_NOT_FOUND", "countdown not found")
}

impl Plans {
    /// Reads who a countdown is shared with: nobody unless said otherwise.
    async fn countdown_sharing(
        &self,
        user_id: Uuid,
        sharing: CountdownSharing,
        shared_with: &[String],
    ) -> AppResult<(CountdownSharing, Vec<Uuid>)> {
        match sharing {
            CountdownSharing::COUNTDOWN_SHARING_CHOSEN => {
                let friends = input::people(shared_with, user_id)?;
                ensure!(
                    !friends.is_empty(),
                    invalid,
                    "SHARED_WITH_REQUIRED",
                    "choose at least one friend to share with"
                );
                ensure!(
                    friends.len() <= MAX_SHARED,
                    invalid,
                    "TOO_MANY_SHARED",
                    "share with at most {MAX_SHARED} friends"
                );
                people::require_friends(&self.core, user_id, &friends).await?;
                Ok((sharing, friends))
            }
            CountdownSharing::COUNTDOWN_SHARING_PROFILE => Ok((sharing, Vec::new())),
            _ => Ok((CountdownSharing::COUNTDOWN_SHARING_PRIVATE, Vec::new())),
        }
    }

    /// A countdown can be pinned while fewer than [`MAX_PINNED`] others are.
    async fn ensure_can_pin(&self, user_id: Uuid, except: Option<Uuid>) -> AppResult<()> {
        let pinned: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM plan_countdowns WHERE user_id = $1 AND sharing = 3 AND id IS DISTINCT FROM $2",
        )
        .bind(user_id)
        .bind(except)
        .fetch_one(&self.core.db)
        .await?;
        ensure!(
            (pinned as usize) < MAX_PINNED,
            precondition,
            "TOO_MANY_PINNED",
            "at most {MAX_PINNED} countdowns can be pinned to the profile"
        );
        Ok(())
    }

    /// The countdown as its owner sees it, with who it is shared with.
    async fn countdown_for_owner(&self, row: &CountdownRow, zone: Tz) -> AppResult<Countdown> {
        let core = &self.core;
        let mut countdown =
            row.to_proto(tokuly_identity::summary(core, row.user_id).await?, dates::day_in(zone, now()));
        countdown.sharing = row.sharing().into();
        countdown.shared_with = tokuly_identity::summaries_ordered(core, &row.shared_ids).await?;
        Ok(countdown)
    }

    pub(crate) async fn create_countdown(&self, user_id: Uuid, req: &CreateCountdownRequest) -> AppResult<Countdown> {
        let core = &self.core;
        let zone = dates::zone_of(core, user_id).await?;
        let now = now();
        let title = countdown_title(&req.title)?;
        let emoji = input::emoji(&req.emoji)?;
        let target_date = input::date(&req.date, "DATE_INVALID")?;
        // A one-off day that has passed is nothing to count down to.
        ensure!(
            req.repeats_yearly || target_date >= dates::day_in(zone, now),
            invalid,
            "DATE_INVALID",
            "the day has already passed"
        );
        let (sharing, shared) = self
            .countdown_sharing(
                user_id,
                req.sharing.as_known().unwrap_or(CountdownSharing::COUNTDOWN_SHARING_UNSPECIFIED),
                &req.shared_with_ids,
            )
            .await?;
        let total: i64 = sqlx::query_scalar("SELECT count(*) FROM plan_countdowns WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&core.db)
            .await?;
        ensure!(total < MAX_COUNTDOWNS, precondition, "TOO_MANY_COUNTDOWNS", "at most {MAX_COUNTDOWNS} countdowns");
        if sharing == CountdownSharing::COUNTDOWN_SHARING_PROFILE {
            self.ensure_can_pin(user_id, None).await?;
        }

        let draft = CountdownRow {
            id: new_id(),
            user_id,
            title,
            emoji,
            target_date,
            repeats_yearly: req.repeats_yearly,
            sharing: sharing as i32 as i16,
            shared_ids: shared,
            created_at: now,
        };
        let row: CountdownRow = sqlx::query_as(concat!(
            "INSERT INTO plan_countdowns AS c (id, user_id, title, emoji, target_date, repeats_yearly, sharing, shared_ids, notify_due_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING ",
            countdown_columns!()
        ))
        .bind(draft.id)
        .bind(user_id)
        .bind(&draft.title)
        .bind(&draft.emoji)
        .bind(draft.target_date)
        .bind(draft.repeats_yearly)
        .bind(draft.sharing)
        .bind(&draft.shared_ids)
        .bind(draft.notify_due(zone, now, self.cfg.morning_hour))
        .fetch_one(&core.db)
        .await?;
        self.countdown_for_owner(&row, zone).await
    }

    pub(crate) async fn update_countdown(&self, user_id: Uuid, req: &UpdateCountdownRequest) -> AppResult<Countdown> {
        let core = &self.core;
        let countdown_id = parse_id(&req.countdown_id)?;
        let zone = dates::zone_of(core, user_id).await?;
        let now = now();
        let mut row: CountdownRow = sqlx::query_as(concat!(
            "SELECT ",
            countdown_columns!(),
            " FROM plan_countdowns c WHERE c.id = $1 AND c.user_id = $2"
        ))
        .bind(countdown_id)
        .bind(user_id)
        .fetch_optional(&core.db)
        .await?
        .ok_or_else(not_found)?;

        if let Some(title) = req.title.as_deref() {
            row.title = countdown_title(title)?;
        }
        if let Some(emoji) = req.emoji.as_deref() {
            row.emoji = input::emoji(emoji)?;
        }
        let (old_date, old_repeats) = (row.target_date, row.repeats_yearly);
        if let Some(date) = req.date.as_deref() {
            row.target_date = input::date(date, "DATE_INVALID")?;
        }
        if let Some(repeats) = req.repeats_yearly {
            row.repeats_yearly = repeats;
        }
        if (row.target_date, row.repeats_yearly) != (old_date, old_repeats) {
            ensure!(
                row.repeats_yearly || row.target_date >= dates::day_in(zone, now),
                invalid,
                "DATE_INVALID",
                "the day has already passed"
            );
        }
        let wanted = req.sharing.as_known().unwrap_or(CountdownSharing::COUNTDOWN_SHARING_UNSPECIFIED);
        if wanted != CountdownSharing::COUNTDOWN_SHARING_UNSPECIFIED {
            let (sharing, shared) = self.countdown_sharing(user_id, wanted, &req.shared_with_ids).await?;
            if sharing == CountdownSharing::COUNTDOWN_SHARING_PROFILE {
                self.ensure_can_pin(user_id, Some(row.id)).await?;
            }
            row.sharing = sharing as i32 as i16;
            row.shared_ids = shared;
        }

        sqlx::query("UPDATE plan_countdowns SET title = $2, emoji = $3, target_date = $4, repeats_yearly = $5, sharing = $6, shared_ids = $7, notify_due_at = $8 WHERE id = $1")
            .bind(row.id)
            .bind(&row.title)
            .bind(&row.emoji)
            .bind(row.target_date)
            .bind(row.repeats_yearly)
            .bind(row.sharing)
            .bind(&row.shared_ids)
            .bind(row.notify_due(zone, now, self.cfg.morning_hour))
            .execute(&core.db)
            .await?;
        self.countdown_for_owner(&row, zone).await
    }

    pub(crate) async fn delete_countdown(&self, user_id: Uuid, countdown_id: Uuid) -> AppResult<()> {
        let deleted = sqlx::query("DELETE FROM plan_countdowns WHERE id = $1 AND user_id = $2")
            .bind(countdown_id)
            .bind(user_id)
            .execute(&self.core.db)
            .await?;
        ensure!(deleted.rows_affected() > 0, not_found, "COUNTDOWN_NOT_FOUND", "countdown not found");
        Ok(())
    }

    /// One countdown: the caller's own, one shared with them, or one a friend
    /// pinned to their profile.
    pub(crate) async fn get_countdown(&self, viewer: Uuid, countdown_id: Uuid) -> AppResult<Countdown> {
        let core = &self.core;
        let row: CountdownRow =
            sqlx::query_as(concat!("SELECT ", countdown_columns!(), " FROM plan_countdowns c WHERE c.id = $1"))
                .bind(countdown_id)
                .fetch_optional(&core.db)
                .await?
                .ok_or_else(not_found)?;
        let zone = dates::zone_of(core, row.user_id).await?;
        if row.user_id == viewer {
            return self.countdown_for_owner(&row, zone).await;
        }
        let shared = match row.sharing() {
            CountdownSharing::COUNTDOWN_SHARING_CHOSEN => row.shared_ids.contains(&viewer),
            CountdownSharing::COUNTDOWN_SHARING_PROFILE => true,
            _ => false,
        };
        // Sharing is among friends: it ends with the friendship.
        ensure!(
            shared && graph::are_friends(core, row.user_id, viewer).await?,
            not_found,
            "COUNTDOWN_NOT_FOUND",
            "countdown not found"
        );
        Ok(row.to_proto(tokuly_identity::summary(core, row.user_id).await?, dates::day_in(zone, now())))
    }

    /// `user_id`'s own countdowns and the ones friends shared with them.
    pub(crate) async fn visible_countdowns(&self, user_id: Uuid) -> AppResult<Vec<CountdownRow>> {
        let core = &self.core;
        let friends = graph::friend_ids(core, user_id).await?;
        Ok(sqlx::query_as(concat!(
            "(SELECT ",
            countdown_columns!(),
            " FROM plan_countdowns c WHERE c.user_id = $1)
             UNION ALL
             (SELECT ",
            countdown_columns!(),
            " FROM plan_countdowns c WHERE c.sharing = 2 AND c.shared_ids @> ARRAY[$1]::uuid[] AND c.user_id = ANY($2) ORDER BY c.id DESC LIMIT $3)"
        ))
        .bind(user_id)
        .bind(&friends)
        .bind(MAX_SHARED_LISTED)
        .fetch_all(&core.db)
        .await?)
    }

    /// The caller's countdowns and the ones shared with them: the nearest
    /// day first, days that have passed last (most recent first).
    pub(crate) async fn list_countdowns(&self, user_id: Uuid, page: &Page) -> AppResult<(Vec<Countdown>, PageInfo)> {
        let core = &self.core;
        let rows = self.visible_countdowns(user_id).await?;
        let owners: Vec<Uuid> = rows.iter().map(|row| row.user_id).collect::<HashSet<_>>().into_iter().collect();
        let zones = dates::zones_of(core, &owners).await?;
        let summaries = tokuly_identity::summaries(core, &owners).await?;
        let now = now();
        let today_of = |owner: Uuid| dates::day_in(zones.get(&owner).copied().unwrap_or(dates::FALLBACK_ZONE), now);

        let mut listed: Vec<(i32, CountdownRow)> = rows
            .into_iter()
            .map(|row| (dates::days_until(row.next_date(today_of(row.user_id)), today_of(row.user_id)), row))
            .collect();
        // Upcoming days ascending, then passed days from the most recent.
        listed.sort_by_key(|(days_left, row)| (*days_left < 0, days_left.abs(), row.id));

        // The order depends on today's date, so pages are cut by position.
        let offset: usize = page.decode()?.unwrap_or(0);
        let limit = page.limit as usize;
        let has_more = listed.len() > offset + limit;
        let info = PageInfo {
            next_cursor: if has_more { encode_cursor(&(offset + limit)) } else { String::new() },
            has_more,
            ..Default::default()
        };

        let mut shared_with: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        let mut countdowns = Vec::new();
        for (_, row) in listed.into_iter().skip(offset).take(limit) {
            let mut countdown =
                row.to_proto(tokuly_identity::users::pick(&summaries, row.user_id), today_of(row.user_id));
            if row.user_id == user_id {
                countdown.sharing = row.sharing().into();
                shared_with.insert(row.id, row.shared_ids.clone());
            }
            countdowns.push(countdown);
        }
        let named: Vec<Uuid> = shared_with.values().flatten().copied().collect::<HashSet<_>>().into_iter().collect();
        let names = tokuly_identity::summaries(core, &named).await?;
        for countdown in &mut countdowns {
            if let Some(ids) = parse_id(&countdown.id).ok().and_then(|id| shared_with.get(&id)) {
                countdown.shared_with = ids.iter().filter_map(|id| names.get(id).cloned()).collect();
            }
        }
        Ok((countdowns, info))
    }

    /// The countdowns `owner` pinned to their profile that still lie ahead,
    /// nearest first, each with the days left.
    pub(crate) async fn pinned_countdowns(&self, owner: Uuid) -> AppResult<Vec<(CountdownRow, i32)>> {
        let core = &self.core;
        let rows: Vec<CountdownRow> = sqlx::query_as(concat!(
            "SELECT ",
            countdown_columns!(),
            " FROM plan_countdowns c WHERE c.user_id = $1 AND c.sharing = 3"
        ))
        .bind(owner)
        .fetch_all(&core.db)
        .await?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let today = dates::day_in(dates::zone_of(core, owner).await?, now());
        let mut pinned: Vec<(CountdownRow, i32)> = rows
            .into_iter()
            .map(|row| {
                let days_left = dates::days_until(row.next_date(today), today);
                (row, days_left)
            })
            .filter(|(_, days_left)| *days_left >= 0)
            .collect();
        pinned.sort_by_key(|(row, days_left)| (*days_left, row.id));
        pinned.truncate(MAX_PINNED);
        Ok(pinned)
    }

    /// Tells owners "today is the day" for the countdowns that reached zero,
    /// and schedules the next occurrence of the ones that repeat. Returns how
    /// many countdowns were handled.
    pub(crate) async fn notify_due_countdowns(&self) -> AppResult<u64> {
        let core = &self.core;
        let mut tx = core.db.begin().await?;
        let due: Vec<CountdownRow> = sqlx::query_as(concat!(
            "SELECT ",
            countdown_columns!(),
            " FROM plan_countdowns c WHERE c.notify_due_at <= now() ORDER BY c.notify_due_at LIMIT 200 FOR UPDATE SKIP LOCKED"
        ))
        .fetch_all(&mut *tx)
        .await?;
        let owners: Vec<Uuid> = due.iter().map(|row| row.user_id).collect();
        let zones = dates::zones_of(core, &owners).await?;
        let now = now();
        for row in &due {
            let zone = zones.get(&row.user_id).copied().unwrap_or(dates::FALLBACK_ZONE);
            let today = dates::day_in(zone, now);
            // After a long pause of the worker the day may be over: say nothing then.
            if row.next_date(today) == today {
                let text = copy::countdown_today(&copy::plan_name(&row.emoji, &row.title));
                let request = NotificationRequest::new(
                    row.user_id,
                    "plans.countdown",
                    text.title,
                    text.body,
                    countdown_path(row.id),
                )
                .collapse(format!("plans.countdown:{}", row.id))
                .data(serde_json::json!({ "countdown_id": row.id }));
                outbound::notify(&mut *tx, &request).await?;
            }
            sqlx::query("UPDATE plan_countdowns SET notify_due_at = $2 WHERE id = $1")
                .bind(row.id)
                .bind(row.notify_due(zone, now, self.cfg.morning_hour))
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(due.len() as u64)
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    const TOKYO: Tz = chrono_tz::Asia::Tokyo;

    fn date(text: &str) -> NaiveDate {
        dates::parse_date(text).unwrap()
    }

    fn tokyo(year: i32, month: u32, day: u32, hour: u32) -> Time {
        TOKYO.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap().with_timezone(&chrono::Utc)
    }

    fn countdown(target: &str, repeats_yearly: bool) -> CountdownRow {
        CountdownRow {
            id: Uuid::nil(),
            user_id: Uuid::nil(),
            title: "文化祭".into(),
            emoji: "🎪".into(),
            target_date: date(target),
            repeats_yearly,
            sharing: 1,
            shared_ids: Vec::new(),
            created_at: tokyo(2026, 1, 1, 0),
        }
    }

    #[test]
    fn a_one_off_day_stays_where_it_is() {
        let festival = countdown("2026-12-24", false);
        assert_eq!(festival.next_date(date("2026-11-22")), date("2026-12-24"));
        assert_eq!(festival.next_date(date("2027-01-05")), date("2026-12-24"));
        assert_eq!(festival.to_proto(Default::default(), date("2026-11-22")).days_left, 32);
        assert_eq!(festival.to_proto(Default::default(), date("2026-12-27")).days_left, -3);
        assert_eq!(festival.date_in(2026), Some(date("2026-12-24")));
        assert_eq!(festival.date_in(2027), None);
    }

    #[test]
    fn a_yearly_day_comes_around() {
        let birthday = countdown("2010-12-24", true);
        assert_eq!(birthday.next_date(date("2026-11-22")), date("2026-12-24"));
        assert_eq!(birthday.next_date(date("2026-12-24")), date("2026-12-24"));
        assert_eq!(birthday.next_date(date("2026-12-25")), date("2027-12-24"));
        let shown = birthday.to_proto(Default::default(), date("2026-12-25"));
        assert_eq!((shown.date.as_str(), shown.next_date.as_str(), shown.days_left), ("2010-12-24", "2027-12-24", 364));
        assert_eq!(birthday.date_in(2030), Some(date("2030-12-24")));
        // Not before it first happened.
        assert_eq!(birthday.date_in(2009), None);
    }

    #[test]
    fn the_owner_is_told_on_the_morning_of_the_day() {
        let festival = countdown("2026-12-24", false);
        assert_eq!(festival.notify_due(TOKYO, tokyo(2026, 11, 22, 12), 8), Some(tokyo(2026, 12, 24, 8)));
        // Created on the day itself, before the morning hour: still told today.
        assert_eq!(festival.notify_due(TOKYO, tokyo(2026, 12, 24, 6), 8), Some(tokyo(2026, 12, 24, 8)));
        // Once told (or created later that day) a one-off has nothing left to announce.
        assert_eq!(festival.notify_due(TOKYO, tokyo(2026, 12, 24, 8), 8), None);
        // A yearly one moves on to next year.
        let birthday = countdown("2010-12-24", true);
        assert_eq!(birthday.notify_due(TOKYO, tokyo(2026, 12, 24, 8), 8), Some(tokyo(2027, 12, 24, 8)));
    }
}
