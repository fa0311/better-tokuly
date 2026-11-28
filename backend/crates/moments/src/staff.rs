//! What admins do with the prompt regions: see today's day of each band and
//! send its prompt now instead of at its drawn time.
//!
//! Sending now is the planner's job with an earlier time: under the day's
//! row lock the day is announced (`days::announce`, with the admin as
//! `sent_by` and the event's actor), and the `moments.prompt` job tells the
//! region as it would have. Once per band and date: a day that went out, or
//! whose time has come and whose job is on its way, is not the admin's to
//! send. A day the job skipped (workers were down) never went out, so it
//! may.
use chrono::NaiveDate;
use tokuly_core::prelude::*;
use tokuly_proto::moments::PromptRegion;

use crate::{
    Moments,
    days::{self, JOB_PROMPT, PromptJob},
    rules,
};

#[derive(sqlx::FromRow)]
struct TodayRow {
    region: String,
    date: NaiveDate,
    announced_at: Option<Time>,
    sent_by: Option<Uuid>,
}

impl TodayRow {
    fn to_proto(&self) -> PromptRegion {
        PromptRegion {
            region: self.region.clone(),
            date: self.date.to_string(),
            sent_at: opt_ts(self.announced_at),
            sent_by: self.sent_by.map(|id| id.to_string()).unwrap_or_default(),
            ..Default::default()
        }
    }
}

impl Moments {
    /// Every band with its day of today, west to east.
    pub(crate) async fn prompt_regions(&self) -> AppResult<Vec<PromptRegion>> {
        let now = now();
        self.draw_days(now).await?;
        let (names, dates): (Vec<&str>, Vec<NaiveDate>) =
            rules::BANDS.iter().map(|band| (band.name, rules::band_date(band, now))).unzip();
        let rows: Vec<TodayRow> = sqlx::query_as(
            "SELECT d.region, d.date, d.announced_at, d.sent_by
             FROM unnest($1::text[], $2::date[]) WITH ORDINALITY AS today (region, date, position)
             JOIN moment_days d ON d.region = today.region AND d.date = today.date
             ORDER BY today.position",
        )
        .bind(&names)
        .bind(&dates)
        .fetch_all(&self.core.db)
        .await?;
        Ok(rows.iter().map(TodayRow::to_proto).collect())
    }

    /// Sends `region`'s prompt of today now, on `admin`'s word.
    pub(crate) async fn send_now(&self, admin: Uuid, region: &str) -> AppResult<PromptRegion> {
        let Some(band) = rules::BANDS.iter().find(|band| band.name == region) else {
            bail!(not_found, "REGION_NOT_FOUND", "no such prompt region");
        };
        let now = now();
        // The planner's groundwork, before the lock: the region, its time
        // zones (who is told) and its day of today.
        self.refresh_regions().await?;
        self.draw_days(now).await?;
        let date = rules::band_date(band, now);

        let mut tx = self.core.db.begin().await?;
        let day: (Uuid, Option<Time>, Option<Time>, Option<Time>) = sqlx::query_as(
            "SELECT id, prompt_at, announced_at, told_at FROM moment_days
             WHERE region = $1 AND date = $2 FOR NO KEY UPDATE",
        )
        .bind(band.name)
        .bind(date)
        .fetch_one(&mut *tx)
        .await?;
        let (day_id, prompt_at, announced_at, told_at) = day;
        let skipped = told_at.is_some();
        let due = prompt_at.is_some_and(|at| at <= now);
        ensure!(
            announced_at.is_none() && (skipped || !due),
            precondition,
            "PROMPT_ALREADY_SENT",
            "today's prompt of {} went out or is on its way",
            band.name
        );
        sqlx::query("UPDATE moment_days SET prompt_at = $2, told_at = NULL, told_through = NULL WHERE id = $1")
            .bind(day_id)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        days::announce(&mut tx, day_id, band.name, date, now, Some(admin)).await?;
        tx.commit().await?;
        tracing::info!(%admin, region = band.name, %date, %day_id, "moments prompt sent now by an admin");

        // The day is out; should the queue fail, the planner's next pass
        // queues the job.
        if let Err(error) = self.core.jobs.enqueue(JOB_PROMPT, &PromptJob { day_id }).await {
            tracing::warn!(%day_id, %error, "moments prompt job not queued; the planner will");
        }
        Ok(TodayRow { region: band.name.to_owned(), date, announced_at: Some(now), sent_by: Some(admin) }.to_proto())
    }
}
