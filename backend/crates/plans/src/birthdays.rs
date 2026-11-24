//! Friends' birthdays, and the setting that shares one's own.
//!
//! Birthdays and who sees them belong to the account: identity's
//! `users.birthdate` and its privacy setting (`tokuly_identity::birthdays`),
//! read through identity's batched API. This domain only offers the switch
//! on its own settings screen ("share with friends"), which writes that
//! setting. Only the month and the day are shown here, whatever the owner
//! shares with friends.
use chrono::{Datelike as _, NaiveDate};
use tokuly_core::prelude::*;
use tokuly_identity::birthdays::Closeness;
use tokuly_proto::{identity::Audience, plans::*};
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
        let privacy = tokuly_identity::settings::privacy_of(&self.core, user_id).await?;
        let shared = tokuly_identity::birthdays::audience_of(&privacy) != Audience::AUDIENCE_NOBODY;
        Ok(PlansSettings { share_birthday: shared, ..Default::default() })
    }

    /// Turning sharing on shows the birthday to friends (an account that
    /// already shows it to everyone keeps that); off hides it from everyone.
    pub(crate) async fn update_plans_settings(
        &self,
        user_id: Uuid,
        share_birthday: Option<bool>,
    ) -> AppResult<PlansSettings> {
        if let Some(share_birthday) = share_birthday {
            let current = self.plans_settings(user_id).await?.share_birthday;
            if share_birthday != current {
                let audience = if share_birthday { Audience::AUDIENCE_FRIENDS } else { Audience::AUDIENCE_NOBODY };
                tokuly_identity::birthdays::set_birthday_audience(&self.core, user_id, audience, None).await?;
            }
        }
        self.plans_settings(user_id).await
    }

    /// The birthdays `viewer` gets to see: those of friends who share theirs.
    pub(crate) async fn shared_birthdays(&self, viewer: Uuid) -> AppResult<Vec<SharedBirthday>> {
        let core = &self.core;
        let hidden = graph::blocked_ids_either(core, viewer).await?;
        let friends: Vec<Uuid> =
            graph::friend_ids(core, viewer).await?.into_iter().filter(|friend| !hidden.contains(friend)).collect();
        let shared = tokuly_identity::birthdays::shared_birthdays(core, &friends, |_| Closeness::Friend).await?;
        let mut birthdays: Vec<SharedBirthday> =
            shared.into_values().map(|b| SharedBirthday { user_id: b.user_id, month: b.month, day: b.day }).collect();
        birthdays.sort_by_key(|b| b.user_id);
        Ok(birthdays)
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
