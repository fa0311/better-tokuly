//! Per-account settings and the defaults by age group.
//!
//! The defaults are decided by the age group on the day it is known — at
//! sign-up (`identity.user.created`) or when an older account gives its
//! birthdate (`identity.user.birthdate_given`) — and stored, never worked out
//! again from today's date: location on for adults, off for minors. An
//! account without a row reads as a minor's defaults. Turning the location
//! on or off oneself is final: age defaults leave it alone from then on.
use std::time::Duration;

use tokuly_core::prelude::*;
use tokuly_proto::moments::{MomentAudience, MomentSettings, UpdateMomentSettingsRequest};

use crate::{Moments, model::audience};

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::FromRow)]
pub(crate) struct Settings {
    pub share_location: bool,
    pub audience: i16,
    pub prompts: bool,
}

impl Settings {
    /// What an account without a row gets: a minor's defaults.
    const UNKNOWN: Self = Self { share_location: false, audience: audience::FRIENDS, prompts: true };

    pub fn to_proto(self) -> MomentSettings {
        MomentSettings {
            share_location: self.share_location,
            audience: audience_enum(self.audience).into(),
            prompts: self.prompts,
            ..Default::default()
        }
    }
}

pub(crate) fn audience_enum(stored: i16) -> MomentAudience {
    if stored == audience::CLOSE_FRIENDS {
        MomentAudience::MOMENT_AUDIENCE_CLOSE_FRIENDS
    } else {
        MomentAudience::MOMENT_AUDIENCE_FRIENDS
    }
}

impl Moments {
    pub(crate) async fn settings_of(&self, user_id: Uuid) -> AppResult<Settings> {
        let stored: Option<Settings> =
            sqlx::query_as("SELECT share_location, audience, prompts FROM moment_settings WHERE user_id = $1")
                .bind(user_id)
                .fetch_optional(&self.core.db)
                .await?;
        Ok(stored.unwrap_or(Settings::UNKNOWN))
    }

    pub(crate) async fn update_settings(
        &self,
        user_id: Uuid,
        request: UpdateMomentSettingsRequest,
    ) -> AppResult<Settings> {
        self.core.cache.rate_limit(&format!("moments-settings:{user_id}"), 30, Duration::from_secs(3600)).await?;
        let audience = match request.audience.as_known() {
            Some(MomentAudience::MOMENT_AUDIENCE_UNSPECIFIED) => None,
            Some(chosen) => Some(chosen as i16),
            None => bail!(invalid, "INVALID_AUDIENCE", "unknown audience"),
        };
        Ok(sqlx::query_as(
            "INSERT INTO moment_settings (user_id, share_location, location_chosen, audience, prompts)
             VALUES ($1, coalesce($2, false), $2 IS NOT NULL, coalesce($3, 1), coalesce($4, true))
             ON CONFLICT (user_id) DO UPDATE SET
                 share_location = coalesce($2, moment_settings.share_location),
                 location_chosen = moment_settings.location_chosen OR $2 IS NOT NULL,
                 audience = coalesce($3, moment_settings.audience),
                 prompts = coalesce($4, moment_settings.prompts)
             RETURNING share_location, audience, prompts",
        )
        .bind(user_id)
        .bind(request.share_location)
        .bind(audience)
        .bind(request.prompts)
        .fetch_one(&self.core.db)
        .await?)
    }

    /// Applies the defaults of an age group, unless the person chose the
    /// location themselves. Idempotent.
    pub(crate) async fn apply_age_defaults(&self, user_id: Uuid, minor: bool) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO moment_settings (user_id, share_location)
             SELECT id, NOT $2 FROM users WHERE id = $1 AND deleted_at IS NULL
             ON CONFLICT (user_id) DO UPDATE SET share_location = NOT $2
             WHERE NOT moment_settings.location_chosen AND moment_settings.share_location = $2",
        )
        .bind(user_id)
        .bind(minor)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }
}
