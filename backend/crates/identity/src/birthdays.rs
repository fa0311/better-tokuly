//! Who sees a birthday.
//!
//! The birthday is the account's (`users.birthdate`, given at sign-up for
//! the minimum-age check). Whether anyone else sees it is a privacy setting
//! (`PrivacySettings.birthday_visible_to` / `birthday_show_year`): nobody
//! (the default), friends or everyone, the month and day or the full date.
//! An account under 18 cannot show the year to everyone: it gives the age
//! away. Every place that shows a birthday — the profile, friends'
//! calendars and reminders in the plans domain — reads it through
//! [`shared_birthdays`] or [`crate::Identity::visible_birthdays`], so the one
//! setting holds everywhere.
use std::collections::HashMap;

use chrono::{Datelike as _, NaiveDate, Utc};
use tokuly_core::{events, prelude::*};
use tokuly_proto::identity::{Audience, PrivacySettings, Relationship};

use crate::{Identity, maintenance::UserRef, settings};

/// A birthday as someone may see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedBirthday {
    pub user_id: Uuid,
    pub month: u32,
    pub day: u32,
    /// Only when the owner shows the year to this viewer.
    pub year: Option<i32>,
}

/// What the viewer is to the owner of a birthday.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Closeness {
    Owner,
    Friend,
    Other,
}

/// How a birthday is shared, from the privacy settings. An audience never
/// set (older accounts) is nobody.
pub fn audience_of(privacy: &PrivacySettings) -> Audience {
    match privacy.birthday_visible_to.as_known() {
        Some(audience @ (Audience::AUDIENCE_EVERYONE | Audience::AUDIENCE_FRIENDS)) => audience,
        _ => Audience::AUDIENCE_NOBODY,
    }
}

/// The birthday of one owner as a viewer of the given closeness sees it.
pub fn share(
    user_id: Uuid,
    birthdate: NaiveDate,
    privacy: &PrivacySettings,
    closeness: Closeness,
) -> Option<SharedBirthday> {
    let visible = match closeness {
        Closeness::Owner => true,
        Closeness::Friend => audience_of(privacy) != Audience::AUDIENCE_NOBODY,
        Closeness::Other => audience_of(privacy) == Audience::AUDIENCE_EVERYONE,
    };
    if !visible {
        return None;
    }
    let year = (closeness == Closeness::Owner || privacy.birthday_show_year).then(|| birthdate.year());
    Some(SharedBirthday { user_id, month: birthdate.month(), day: birthdate.day(), year })
}

/// The birthdays of `owners` that a viewer may see, in one query; owners
/// without a birthday, deleted or suspended accounts, and those who do not
/// share with this viewer are absent. `closeness` says what the viewer is
/// to each owner (the caller knows friendships; blocks must already be
/// filtered out by the caller).
pub async fn shared_birthdays(
    core: &Core,
    owners: &[Uuid],
    closeness: impl Fn(Uuid) -> Closeness,
) -> AppResult<HashMap<Uuid, SharedBirthday>> {
    if owners.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(Uuid, NaiveDate, serde_json::Value)> = sqlx::query_as(
        "SELECT id, birthdate, privacy FROM users
         WHERE id = ANY($1) AND birthdate IS NOT NULL AND deleted_at IS NULL AND banned_at IS NULL",
    )
    .bind(owners)
    .fetch_all(&core.db)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, birthdate, privacy)| {
            share(id, birthdate, &settings::privacy_from_json(&privacy), closeness(id)).map(|b| (id, b))
        })
        .collect())
}

/// Changes who sees `user_id`'s birthday, for a domain that offers the
/// switch on its own screen (the plans calendar). `show_year` of `None`
/// keeps the stored choice. Emits `identity.user.privacy_changed` when
/// something changed.
pub async fn set_birthday_audience(
    core: &Core,
    user_id: Uuid,
    audience: Audience,
    show_year: Option<bool>,
) -> AppResult<()> {
    ensure!(
        matches!(audience, Audience::AUDIENCE_EVERYONE | Audience::AUDIENCE_FRIENDS | Audience::AUDIENCE_NOBODY),
        invalid,
        "SETTINGS_INVALID",
        "a birthday is shown to everyone, friends or nobody"
    );
    let mut tx = core.db.begin().await?;
    let row: Option<(serde_json::Value, Option<NaiveDate>)> =
        sqlx::query_as("SELECT privacy, birthdate FROM users WHERE id = $1 AND deleted_at IS NULL FOR UPDATE")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    let (stored, birthdate) = row.or_not_found("USER_NOT_FOUND", "user not found")?;
    let mut privacy = settings::privacy_from_json(&stored);
    let before = (audience_of(&privacy), privacy.birthday_show_year);
    privacy.birthday_visible_to = audience.into();
    if let Some(show_year) = show_year {
        privacy.birthday_show_year = show_year;
    }
    check_year_rule(&privacy, birthdate, Utc::now().date_naive())?;
    if before == (audience, privacy.birthday_show_year) {
        return Ok(());
    }
    let json =
        serde_json::to_value(&privacy).map_err(|e| AppError::internal(anyhow::anyhow!("settings encode: {e}")))?;
    sqlx::query("UPDATE users SET privacy = $2 WHERE id = $1").bind(user_id).bind(json).execute(&mut *tx).await?;
    events::emit(&mut *tx, "identity.user.privacy_changed", Some(user_id), &UserRef { user_id }).await?;
    tx.commit().await?;
    Ok(())
}

/// The year of a minor's birthday (under [`crate::age::ADULT_AGE`]) is not shown to everyone.
pub(crate) fn check_year_rule(
    privacy: &PrivacySettings,
    birthdate: Option<NaiveDate>,
    today: NaiveDate,
) -> AppResult<()> {
    let minor = birthdate.is_some() && crate::age::minor_on(birthdate, today);
    ensure!(
        !(minor && privacy.birthday_show_year && audience_of(privacy) == Audience::AUDIENCE_EVERYONE),
        invalid,
        "BIRTHDAY_YEAR_RESTRICTED",
        "under 18 the year of your birthday is shown to friends at most"
    );
    Ok(())
}

impl Identity {
    /// The birthdays of `owners` that `viewer` may see (`None`: someone
    /// signed out), friendships and blocks taken from the registered
    /// profile extensions: a pair in a block sees nothing of each other.
    pub async fn visible_birthdays(
        &self,
        viewer: Option<Uuid>,
        owners: &[Uuid],
    ) -> AppResult<HashMap<Uuid, SharedBirthday>> {
        let (relationships, hidden) = match viewer {
            Some(viewer) => (self.relationships_of(viewer, owners).await?, self.hidden_among(viewer, owners).await?),
            None => Default::default(),
        };
        let visible: Vec<Uuid> = owners.iter().copied().filter(|id| !hidden.contains(id)).collect();
        shared_birthdays(&self.core, &visible, |owner| {
            if Some(owner) == viewer {
                Closeness::Owner
            } else if relationships.get(&owner).is_some_and(|(r, _)| *r == Relationship::RELATIONSHIP_FRIENDS) {
                Closeness::Friend
            } else {
                Closeness::Other
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn privacy(audience: Audience, show_year: bool) -> PrivacySettings {
        PrivacySettings {
            birthday_visible_to: audience.into(),
            birthday_show_year: show_year,
            ..settings::default_privacy()
        }
    }

    #[test]
    fn birthdays_follow_audience_and_year_choice() {
        let id = Uuid::now_v7();
        let date = NaiveDate::from_ymd_opt(2008, 7, 7).unwrap();
        let friends = privacy(Audience::AUDIENCE_FRIENDS, false);
        assert_eq!(share(id, date, &friends, Closeness::Other), None);
        assert_eq!(share(id, date, &friends, Closeness::Friend).unwrap().year, None);
        assert_eq!(share(id, date, &friends, Closeness::Owner).unwrap().year, Some(2008));
        let everyone = privacy(Audience::AUDIENCE_EVERYONE, true);
        let seen = share(id, date, &everyone, Closeness::Other).unwrap();
        assert_eq!((seen.month, seen.day, seen.year), (7, 7, Some(2008)));
        // Never set, or set to something that is not offered, is nobody.
        let unset =
            PrivacySettings { birthday_visible_to: Audience::AUDIENCE_UNSPECIFIED.into(), ..Default::default() };
        assert_eq!(share(id, date, &unset, Closeness::Friend), None);
        let fof = privacy(Audience::AUDIENCE_FRIENDS_OF_FRIENDS, false);
        assert_eq!(share(id, date, &fof, Closeness::Friend), None);
    }

    #[test]
    fn minors_do_not_show_the_year_to_everyone() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 25).unwrap();
        let minor = Some(NaiveDate::from_ymd_opt(2010, 1, 1).unwrap());
        let adult = Some(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap());
        let open = privacy(Audience::AUDIENCE_EVERYONE, true);
        assert_eq!(check_year_rule(&open, minor, today).unwrap_err().reason, "BIRTHDAY_YEAR_RESTRICTED");
        assert!(check_year_rule(&open, adult, today).is_ok());
        assert!(check_year_rule(&privacy(Audience::AUDIENCE_FRIENDS, true), minor, today).is_ok());
        assert!(check_year_rule(&privacy(Audience::AUDIENCE_EVERYONE, false), minor, today).is_ok());
    }
}
