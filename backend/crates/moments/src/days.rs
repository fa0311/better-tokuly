//! The prompt regions' days, and everyone's own day.
//!
//! A region is a band of UTC offsets (`rules::BANDS`); the planner (every
//! `TOKULY_MOMENTS_WORKER_INTERVAL_SECONDS`) keeps every time zone mapped to
//! its band by its offset right now (`moment_zones`), draws today and
//! tomorrow for each band (`rules::draw_prompt`; the first draw of a date
//! wins) and queues the `moments.prompt` job for every day that is due and
//! not yet fully told. The job is where exactly-once lives:
//!
//! 1. **Announce**, under the day's row lock: `announced_at` is set once,
//!    the region's current day moves to it and `moments.prompt.sent` is
//!    emitted, all in one transaction. A day whose time passed more than
//!    [`MAX_DELAY`] ago (workers were down) or that a later date overtook is
//!    skipped instead: nobody is woken up at night for yesterday's prompt.
//! 2. **Tell**, one page of the region's accounts at a time (ID order, under
//!    the same lock): each account's own day (`moment_people`) moves to this
//!    one if it is of a later date than theirs, and those it moved for are
//!    notified; both commit with the cursor `told_through`, so a job that
//!    dies, runs twice or runs in two processes at once tells nobody twice
//!    and nobody never.
//!
//! An account's day only ever moves to a later date: one day, one prompt and
//! one moment per date, wherever its time zone moves in between. A new time
//! zone counts from the next prompt that reaches the account there; until
//! then the day is the one it had. The one other way into a day is posting
//! while the region's prompt is still on its way to the account
//! ([`Moments::today_of`]).
use std::time::{Duration, Instant};

use chrono::{Days, Duration as Span, NaiveDate};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, PgExecutor};
use tokuly_core::{events, outbound, prelude::*};
use tokuly_proto::moments::{MomentDay, moments_event::Event as Push};

use crate::{
    Moments, copy,
    events::{PROMPT_SENT, PromptSent},
    model::{DayRow, Today},
    rules,
};

/// The job that sends a region's prompt.
pub const JOB_PROMPT: &str = "moments.prompt";
/// Accounts told per transaction.
const TELL_BATCH: i64 = 500;
/// A prompt that could not go out within this long after its time is skipped.
pub const MAX_DELAY: Span = Span::hours(1);
/// How often the planner maps the time zones to their bands again.
const REGION_REFRESH: Duration = Duration::from_secs(600);

/// Payload of [`JOB_PROMPT`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptJob {
    pub day_id: Uuid,
}

/// What [`Moments::send_prompt`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sent {
    /// The day went out (now or before); this many accounts were told by this call.
    Told(usize),
    /// Too late, or a later day went out first: the day never starts.
    Skipped,
    /// No such day, or it is not due.
    Nothing,
}

impl Moments {
    /// One pass of the planner: regions, days ahead, prompts due. Returns
    /// the days it queued.
    pub async fn plan(&self) -> AppResult<Vec<Uuid>> {
        let refresh = {
            let mut last = self.regions_refreshed.lock().expect("not poisoned");
            let due = last.is_none_or(|at| at.elapsed() >= REGION_REFRESH);
            if due {
                *last = Some(Instant::now());
            }
            due
        };
        if refresh {
            self.refresh_regions().await?;
        }
        self.draw_days(now()).await?;
        let due = self.due_days().await?;
        for day_id in &due {
            self.core.jobs.enqueue_unique(JOB_PROMPT, &day_id.to_string(), &PromptJob { day_id: *day_id }).await?;
        }
        Ok(due)
    }

    /// Makes sure every band is a region and maps every time zone to the
    /// band of its offset right now.
    pub async fn refresh_regions(&self) -> AppResult<()> {
        let bands: Vec<&str> = rules::BANDS.iter().map(|band| band.name).collect();
        sqlx::query("INSERT INTO moment_regions (region) SELECT unnest($1::text[]) ON CONFLICT DO NOTHING")
            .bind(&bands)
            .execute(&self.core.db)
            .await?;
        let now = now();
        let (zones, regions): (Vec<&str>, Vec<&str>) = chrono_tz::TZ_VARIANTS
            .iter()
            .map(|zone| (zone.name(), rules::band_of(rules::offset_minutes(*zone, now)).name))
            .unzip();
        sqlx::query(
            "INSERT INTO moment_zones (time_zone, region)
             SELECT * FROM unnest($1::text[], $2::text[])
             ON CONFLICT (time_zone) DO UPDATE SET region = excluded.region
             WHERE moment_zones.region <> excluded.region",
        )
        .bind(&zones)
        .bind(&regions)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    /// Draws the today and tomorrow of every band that has none yet.
    pub async fn draw_days(&self, now: Time) -> AppResult<()> {
        let (names, dates, prompts) = self.draw(now);
        sqlx::query(
            "INSERT INTO moment_days (region, date, prompt_at)
             SELECT * FROM unnest($1::text[], $2::date[], $3::timestamptz[])
             ON CONFLICT (region, date) DO NOTHING",
        )
        .bind(&names)
        .bind(&dates)
        .bind(&prompts)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    /// Today and tomorrow of each band, with their prompt times drawn.
    fn draw(&self, now: Time) -> (Vec<&'static str>, Vec<NaiveDate>, Vec<Option<Time>>) {
        let (mut names, mut dates, mut prompts) = (Vec::new(), Vec::new(), Vec::new());
        let (from, until) = (self.cfg.prompt_from_hour, self.cfg.prompt_until_hour);
        let mut rng = rand::rng();
        for band in &rules::BANDS {
            let today = rules::band_date(band, now);
            for date in [today, today + Days::new(1)] {
                prompts.push(rules::draw_prompt(band, date, from, until, now, &mut rng));
                names.push(band.name);
                dates.push(date);
            }
        }
        (names, dates, prompts)
    }

    /// Days whose time has come and that are not fully told yet.
    pub async fn due_days(&self) -> AppResult<Vec<Uuid>> {
        Ok(sqlx::query_scalar(
            "SELECT id FROM moment_days WHERE prompt_at <= now() AND told_at IS NULL ORDER BY prompt_at LIMIT 200",
        )
        .fetch_all(&self.core.db)
        .await?)
    }

    /// The `moments.prompt` job: announces the day if it has not been, then
    /// tells the region (see the module documentation).
    pub async fn send_prompt(&self, day_id: Uuid) -> AppResult<Sent> {
        let core = &self.core;
        let mut tx = core.db.begin().await?;
        let day: Option<(String, NaiveDate, Option<Time>, Option<Time>, Option<Time>)> = sqlx::query_as(
            // NO KEY UPDATE: moments and accounts referring to the day go on
            // being written while it is told.
            "SELECT region, date, prompt_at, announced_at, told_at FROM moment_days WHERE id = $1 FOR NO KEY UPDATE",
        )
        .bind(day_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((region, date, Some(prompt_at), announced_at, told_at)) = day else { return Ok(Sent::Nothing) };
        let now = now();
        if prompt_at > now {
            return Ok(Sent::Nothing);
        }
        if announced_at.is_none() {
            if told_at.is_some() {
                return Ok(Sent::Skipped);
            }
            let overtaken: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM moment_days WHERE region = $1 AND date > $2 AND announced_at IS NOT NULL)",
            )
            .bind(&region)
            .bind(date)
            .fetch_one(&mut *tx)
            .await?;
            if overtaken || now - prompt_at > MAX_DELAY {
                sqlx::query("UPDATE moment_days SET told_at = now() WHERE id = $1")
                    .bind(day_id)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                tracing::warn!(%day_id, %region, %date, overtaken, "moments prompt skipped");
                return Ok(Sent::Skipped);
            }
            announce(&mut tx, day_id, &region, date, now, None).await?;
        }
        tx.commit().await?;
        let announced = DayRow { id: day_id, region, date, announced_at: announced_at.unwrap_or(now) };
        Ok(Sent::Told(self.tell(&announced).await?))
    }

    /// Moves the region's accounts to the day, page by page from where an
    /// earlier run stopped, and tells those it moved who take prompts.
    async fn tell(&self, day: &DayRow) -> AppResult<usize> {
        #[derive(sqlx::FromRow)]
        struct Member {
            id: Uuid,
            birthdate: Option<NaiveDate>,
            time_zone: String,
            /// Not suspended, and prompts not turned off.
            prompted: bool,
        }
        let core = &self.core;
        let window_minutes = (self.cfg.window_seconds + 59) / 60;
        let shown = self.day_view(day);
        let mut told_total = 0;
        loop {
            let mut tx = core.db.begin().await?;
            let (cursor, told_at): (Option<Uuid>, Option<Time>) =
                sqlx::query_as("SELECT told_through, told_at FROM moment_days WHERE id = $1 FOR NO KEY UPDATE")
                    .bind(day.id)
                    .fetch_one(&mut *tx)
                    .await?;
            if told_at.is_some() {
                return Ok(told_total);
            }
            let page: Vec<Member> = sqlx::query_as(
                "SELECT u.id, u.birthdate, u.time_zone, u.banned_at IS NULL AND coalesce(s.prompts, true) AS prompted
                 FROM users u
                 JOIN moment_zones z ON z.time_zone = u.time_zone
                 LEFT JOIN moment_settings s ON s.user_id = u.id
                 WHERE z.region = $1 AND u.deleted_at IS NULL AND NOT u.bot AND ($2::uuid IS NULL OR u.id > $2)
                 ORDER BY u.id LIMIT $3",
            )
            .bind(&day.region)
            .bind(cursor)
            .bind(TELL_BATCH)
            .fetch_all(&mut *tx)
            .await?;
            let Some(last) = page.last().map(|member| member.id) else {
                sqlx::query("UPDATE moment_days SET told_at = now() WHERE id = $1")
                    .bind(day.id)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                return Ok(told_total);
            };
            let ids: Vec<Uuid> = page.iter().map(|member| member.id).collect();
            let moved: Vec<Uuid> = sqlx::query_scalar(
                "INSERT INTO moment_people (user_id, day_id, date)
                 SELECT unnest($1::uuid[]), $2, $3
                 ON CONFLICT (user_id) DO UPDATE SET day_id = excluded.day_id, date = excluded.date
                 WHERE moment_people.date < excluded.date
                 RETURNING user_id",
            )
            .bind(&ids)
            .bind(day.id)
            .bind(day.date)
            .fetch_all(&mut *tx)
            .await?;
            // An account under the minimum age signs in only to delete itself.
            let told: Vec<Uuid> = page
                .iter()
                .filter(|member| member.prompted && moved.contains(&member.id))
                .filter(|member| !self.identity.too_young(member.birthdate, &member.time_zone))
                .map(|member| member.id)
                .collect();
            for user_id in &told {
                outbound::notify(&mut *tx, &copy::prompt(*user_id, window_minutes)).await?;
            }
            sqlx::query("UPDATE moment_days SET told_through = $2 WHERE id = $1")
                .bind(day.id)
                .bind(last)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            told_total += told.len();
            self.push(told, Push::Prompt(Box::new(shown.clone()))).await;
        }
    }

    /// An account's day as far as the account itself is concerned: the
    /// region's current day while its prompt is still on the way to them
    /// (of a later date than their own day, and the fan-out has not passed
    /// them yet), otherwise their own day. `None` before any prompt reached
    /// them.
    pub(crate) async fn today_of<'e>(&self, executor: impl PgExecutor<'e>, user_id: Uuid) -> AppResult<Option<Today>> {
        Ok(sqlx::query_as(
            "SELECT * FROM (
                 SELECT d.id, d.region, d.date, d.announced_at, true AS fixed
                 FROM moment_people p JOIN moment_days d ON d.id = p.day_id
                 WHERE p.user_id = $1
                 UNION ALL
                 SELECT d.id, d.region, d.date, d.announced_at, false
                 FROM users u
                 JOIN moment_zones z ON z.time_zone = u.time_zone
                 JOIN moment_regions r ON r.region = z.region
                 JOIN moment_days d ON d.id = r.current_day_id
                 WHERE u.id = $1 AND d.told_at IS NULL AND (d.told_through IS NULL OR d.told_through < u.id)
             ) today
             ORDER BY date DESC, fixed DESC LIMIT 1",
        )
        .bind(user_id)
        .fetch_optional(executor)
        .await?)
    }

    /// Fixes the account's day for posting: [`Self::today_of`], made its
    /// own day if it was not yet.
    pub(crate) async fn fix_today(&self, tx: &mut PgConnection, user_id: Uuid) -> AppResult<Option<DayRow>> {
        let Some(today) = self.today_of(&mut *tx, user_id).await? else { return Ok(None) };
        if today.fixed {
            return Ok(Some(today.day));
        }
        let moved: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO moment_people (user_id, day_id, date) VALUES ($1, $2, $3)
             ON CONFLICT (user_id) DO UPDATE SET day_id = excluded.day_id, date = excluded.date
             WHERE moment_people.date < excluded.date
             RETURNING user_id",
        )
        .bind(user_id)
        .bind(today.day.id)
        .bind(today.day.date)
        .fetch_optional(&mut *tx)
        .await?;
        if moved.is_some() {
            return Ok(Some(today.day));
        }
        // The prompt reached them in the meantime: their day is what it set.
        Ok(self.today_of(&mut *tx, user_id).await?.map(|today| today.day))
    }

    pub(crate) fn day_view(&self, day: &DayRow) -> MomentDay {
        MomentDay {
            date: day.date.to_string(),
            region: day.region.clone(),
            prompt_at: ts(day.announced_at),
            on_time_until: ts(rules::on_time_until(day.announced_at, self.cfg.window_seconds)),
            ..Default::default()
        }
    }
}

/// Starts a day, on the transaction that holds its row lock: it is
/// announced (by `sent_by` when an admin sent it now), becomes its region's
/// current day and `moments.prompt.sent` goes out with it.
pub(crate) async fn announce(
    tx: &mut PgConnection,
    day_id: Uuid,
    region: &str,
    date: NaiveDate,
    now: Time,
    sent_by: Option<Uuid>,
) -> AppResult<()> {
    sqlx::query("UPDATE moment_days SET announced_at = $2, sent_by = $3 WHERE id = $1")
        .bind(day_id)
        .bind(now)
        .bind(sent_by)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE moment_regions SET current_day_id = $1 WHERE region = $2")
        .bind(day_id)
        .bind(region)
        .execute(&mut *tx)
        .await?;
    events::emit(
        &mut *tx,
        PROMPT_SENT,
        sent_by,
        &PromptSent { day_id, region: region.to_owned(), date, prompt_at: now },
    )
    .await
}
