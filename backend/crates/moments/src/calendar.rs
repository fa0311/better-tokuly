//! The owner's month: one moment per day, by the date of its prompt.
use std::time::Duration;

use chrono::{Months, NaiveDate};
use tokuly_core::prelude::*;
use tokuly_proto::moments::{CalendarDay, GetCalendarResponse};

use crate::{
    Moments,
    model::{MOMENT_COLUMNS, MomentRow},
};

impl Moments {
    pub(crate) async fn calendar(&self, user_id: Uuid, year: i32, month: i32) -> AppResult<GetCalendarResponse> {
        self.core.cache.rate_limit(&format!("moments-read:{user_id}"), 240, Duration::from_secs(60)).await?;
        let first = u32::try_from(month)
            .ok()
            .filter(|_| (1970..=9999).contains(&year))
            .and_then(|month| NaiveDate::from_ymd_opt(year, month, 1))
            .ok_or_else(|| AppError::invalid("INVALID_MONTH", "a year and a month (1-12) are required"))?;
        let next = first + Months::new(1);
        let rows: Vec<MomentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {MOMENT_COLUMNS} FROM moments m JOIN moment_days d ON d.id = m.day_id
             WHERE m.user_id = $1 AND m.deleted_at IS NULL AND d.date >= $2 AND d.date < $3
             ORDER BY d.date, m.posted_at"
        )))
        .bind(user_id)
        .bind(first)
        .bind(next)
        .fetch_all(&self.core.db)
        .await?;
        Ok(GetCalendarResponse {
            posted_count: rows.len() as i32,
            on_time_count: rows.iter().filter(|m| m.late_seconds == 0).count() as i32,
            days: rows
                .iter()
                .map(|m| CalendarDay {
                    date: m.date.to_string(),
                    moment_id: m.id.to_string(),
                    back: m.back().into(),
                    late_seconds: m.late_seconds,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })
    }
}
