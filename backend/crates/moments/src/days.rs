//! A region's days: drawn ahead, announced once, told to everyone once.
//!
//! The planner (every `TOKULY_MOMENTS_WORKER_INTERVAL_SECONDS`) keeps
//! `moment_regions` in step with the time zones accounts use, draws today
//! and tomorrow for each region (`rules::draw_prompt`; the first draw of a
//! date wins) and queues the `moments.prompt` job for every day that is due
//! and not yet fully told. The job is where exactly-once lives:
//!
//! 1. **Announce**, under the day's row lock: `announced_at` is set once,
//!    the region's current day moves to it and `moments.prompt.sent` is
//!    emitted, all in one transaction. A day whose time passed more than
//!    [`MAX_DELAY`] ago (workers were down) or that a later date overtook is
//!    skipped instead: nobody is woken up at night for yesterday's prompt.
//! 2. **Tell**, one page of accounts at a time (ID order, under the same
//!    lock): the page's notifications and the cursor `told_through` commit
//!    together, so a job that dies, runs twice or runs in two processes at
//!    once tells nobody twice and nobody never.
use std::time::{Duration, Instant};

use chrono::{Days, Duration as Span, NaiveDate};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use sqlx::PgExecutor;
use tokuly_core::{events, outbound, prelude::*};
use tokuly_proto::moments::{MomentDay, moments_event::Event as Push};

use crate::{
    Moments, copy,
    events::{PROMPT_SENT, PromptSent},
    model::DayRow,
    rules,
};

/// The job that sends a region's prompt.
pub const JOB_PROMPT: &str = "moments.prompt";
/// Accounts told per transaction.
const TELL_BATCH: i64 = 500;
/// A prompt that could not go out within this long after its time is skipped.
pub const MAX_DELAY: Span = Span::hours(1);
/// How often the planner looks for new regions among the accounts.
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

    /// Adds the time zones accounts use as regions.
    pub async fn refresh_regions(&self) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO moment_regions (region)
             SELECT DISTINCT time_zone FROM users WHERE deleted_at IS NULL AND NOT bot
             ON CONFLICT DO NOTHING",
        )
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    /// Draws the local today and tomorrow of every region that has none yet.
    pub async fn draw_days(&self, now: Time) -> AppResult<()> {
        let regions: Vec<String> =
            sqlx::query_scalar("SELECT region FROM moment_regions").fetch_all(&self.core.db).await?;
        let (names, dates, prompts) = self.draw(regions, now);
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

    /// Today and tomorrow of each region, with their prompt times drawn.
    fn draw(&self, regions: Vec<String>, now: Time) -> (Vec<String>, Vec<NaiveDate>, Vec<Option<Time>>) {
        let (mut names, mut dates, mut prompts) = (Vec::new(), Vec::new(), Vec::new());
        let mut rng = rand::rng();
        for name in regions {
            let Ok(region) = name.parse::<Tz>() else { continue };
            let today = rules::local_date(region, now);
            for date in [today, today + Days::new(1)] {
                let (from, until) = (self.cfg.prompt_from_hour, self.cfg.prompt_until_hour);
                prompts.push(rules::draw_prompt(region, date, from, until, now, &mut rng));
                names.push(name.clone());
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
            "SELECT region, date, prompt_at, announced_at, told_at FROM moment_days WHERE id = $1 FOR UPDATE",
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
            sqlx::query("UPDATE moment_days SET announced_at = $2 WHERE id = $1")
                .bind(day_id)
                .bind(now)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE moment_regions SET current_day_id = $1 WHERE region = $2")
                .bind(day_id)
                .bind(&region)
                .execute(&mut *tx)
                .await?;
            events::emit(
                &mut *tx,
                PROMPT_SENT,
                None,
                &PromptSent { day_id, region: region.clone(), date, prompt_at: now },
            )
            .await?;
        }
        tx.commit().await?;
        let announced = DayRow { id: day_id, region, date, announced_at: announced_at.unwrap_or(now) };
        Ok(Sent::Told(self.tell(&announced).await?))
    }

    /// Tells the region's accounts that take prompts, page by page, from
    /// where an earlier run stopped.
    async fn tell(&self, day: &DayRow) -> AppResult<usize> {
        let core = &self.core;
        let window_minutes = (self.cfg.window_seconds + 59) / 60;
        let shown = self.day_view(day);
        let mut told_total = 0;
        loop {
            let mut tx = core.db.begin().await?;
            let (cursor, told_at): (Option<Uuid>, Option<Time>) =
                sqlx::query_as("SELECT told_through, told_at FROM moment_days WHERE id = $1 FOR UPDATE")
                    .bind(day.id)
                    .fetch_one(&mut *tx)
                    .await?;
            if told_at.is_some() {
                return Ok(told_total);
            }
            let page: Vec<(Uuid, Option<NaiveDate>)> = sqlx::query_as(
                "SELECT u.id, u.birthdate FROM users u
                 LEFT JOIN moment_settings s ON s.user_id = u.id
                 WHERE u.time_zone = $1 AND u.deleted_at IS NULL AND u.banned_at IS NULL AND NOT u.bot
                   AND coalesce(s.prompts, true) AND ($2::uuid IS NULL OR u.id > $2)
                 ORDER BY u.id LIMIT $3",
            )
            .bind(&day.region)
            .bind(cursor)
            .bind(TELL_BATCH)
            .fetch_all(&mut *tx)
            .await?;
            let Some(&(last, _)) = page.last() else {
                sqlx::query("UPDATE moment_days SET told_at = now() WHERE id = $1")
                    .bind(day.id)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                return Ok(told_total);
            };
            // An account under the minimum age signs in only to delete itself.
            let told: Vec<Uuid> = page
                .iter()
                .filter(|(_, birthdate)| !self.identity.too_young(*birthdate, &day.region))
                .map(|(id, _)| *id)
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

    /// The day a region is on: its latest prompt that went out.
    pub(crate) async fn current_day<'e>(
        &self,
        executor: impl PgExecutor<'e>,
        region: &str,
    ) -> AppResult<Option<DayRow>> {
        Ok(sqlx::query_as(
            "SELECT d.id, d.region, d.date, d.announced_at
             FROM moment_regions r JOIN moment_days d ON d.id = r.current_day_id
             WHERE r.region = $1",
        )
        .bind(region)
        .fetch_optional(executor)
        .await?)
    }

    /// An account's region: its time zone.
    pub(crate) async fn region_of(&self, user_id: Uuid) -> AppResult<String> {
        Ok(sqlx::query_scalar("SELECT time_zone FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&self.core.db)
            .await?)
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
