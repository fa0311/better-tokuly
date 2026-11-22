//! Friends' birthdays, and the setting that shares one's own.
//!
//! Birthdays come from the account (identity's `users.birthdate`) and are
//! read-only here. Identity has no setting saying whether a birthday may be
//! shown to friends, so this domain keeps one of its own: off unless the
//! user switches it on. Only the month and the day ever leave the server —
//! the year would give a minor's age away.
use chrono::{Datelike as _, NaiveDate};
use tokuly_core::prelude::*;
use tokuly_proto::plans::*;
use tokuly_social::graph;

use crate::{Plans, dates};

/// A birthday as it is shared: whose, and which day of the year.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SharedBirthday {
    pub user_id: Uuid,
    pub month: u32,
    pub day: u32,
}

impl Plans {
    pub(crate) async fn plans_settings(&self, user_id: Uuid) -> AppResult<PlansSettings> {
        let share_birthday: Option<bool> =
            sqlx::query_scalar("SELECT share_birthday FROM plan_settings WHERE user_id = $1")
                .bind(user_id)
                .fetch_optional(&self.core.db)
                .await?;
        Ok(PlansSettings { share_birthday: share_birthday.unwrap_or(false), ..Default::default() })
    }

    pub(crate) async fn update_plans_settings(
        &self,
        user_id: Uuid,
        share_birthday: Option<bool>,
    ) -> AppResult<PlansSettings> {
        if let Some(share_birthday) = share_birthday {
            sqlx::query(
                "INSERT INTO plan_settings (user_id, share_birthday) VALUES ($1, $2)
                 ON CONFLICT (user_id) DO UPDATE SET share_birthday = EXCLUDED.share_birthday, updated_at = now()",
            )
            .bind(user_id)
            .bind(share_birthday)
            .execute(&self.core.db)
            .await?;
        }
        self.plans_settings(user_id).await
    }

    /// The birthdays `viewer` gets to see: those of friends who share theirs.
    pub(crate) async fn shared_birthdays(&self, viewer: Uuid) -> AppResult<Vec<SharedBirthday>> {
        let core = &self.core;
        let hidden = graph::blocked_ids_either(core, viewer).await?;
        let friends: Vec<Uuid> =
            graph::friend_ids(core, viewer).await?.into_iter().filter(|friend| !hidden.contains(friend)).collect();
        let rows: Vec<(Uuid, NaiveDate)> = sqlx::query_as(
            "SELECT u.id, u.birthdate FROM users u JOIN plan_settings s ON s.user_id = u.id AND s.share_birthday
             WHERE u.id = ANY($1) AND u.birthdate IS NOT NULL AND u.deleted_at IS NULL AND u.banned_at IS NULL",
        )
        .bind(&friends)
        .fetch_all(&core.db)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(user_id, birthdate)| SharedBirthday { user_id, month: birthdate.month(), day: birthdate.day() })
            .collect())
    }

    /// `user_id`'s own birthday, when their account has one.
    pub(crate) async fn own_birthday(&self, user_id: Uuid) -> AppResult<Option<SharedBirthday>> {
        let birthdate: Option<Option<NaiveDate>> = sqlx::query_scalar("SELECT birthdate FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&self.core.db)
            .await?;
        Ok(birthdate.flatten().map(|birthdate| SharedBirthday {
            user_id,
            month: birthdate.month(),
            day: birthdate.day(),
        }))
    }

    /// Friends' birthdays, the next one first.
    pub(crate) async fn list_birthdays(&self, viewer: Uuid) -> AppResult<Vec<Birthday>> {
        let core = &self.core;
        let shared = self.shared_birthdays(viewer).await?;
        let users: Vec<Uuid> = shared.iter().map(|birthday| birthday.user_id).collect();
        let summaries = tokuly_identity::summaries(core, &users).await?;
        let today = dates::day_in(dates::zone_of(core, viewer).await?, now());
        let mut birthdays: Vec<(NaiveDate, SharedBirthday)> = shared
            .into_iter()
            .map(|birthday| (dates::next_yearly(birthday.month, birthday.day, today), birthday))
            .collect();
        birthdays.sort_by_key(|(next, birthday)| (*next, birthday.user_id));
        Ok(birthdays
            .into_iter()
            .map(|(next, birthday)| Birthday {
                user: tokuly_identity::users::pick(&summaries, birthday.user_id).into(),
                month: birthday.month as i32,
                day: birthday.day as i32,
                next_date: dates::format_date(next),
                days_left: dates::days_until(next, today),
                ..Default::default()
            })
            .collect())
    }
}
