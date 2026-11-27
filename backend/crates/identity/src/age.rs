//! Who counts as a minor, for every domain.
//!
//! Two questions, kept apart:
//!
//! - **Defaults** (the privacy settings of [`crate::settings::minor_defaults`],
//!   a locked account, anything else a domain starts a minor with) are decided
//!   once, by the age on the day the birthdate is given — at sign-up, or later
//!   through `SetBirthdate` — and stored. They are never recomputed from
//!   today's date: whoever turns 18 keeps them until they change them.
//!   `identity.user.created` and `identity.user.birthdate_given` say which
//!   age group that was ([`crate::maintenance::BirthdateGiven`]).
//! - **Limits** (what nobody under 18 may do or choose) follow the age today:
//!   [`is_minor`], [`minors_among`]. An account that has not given its
//!   birthdate yet counts as a minor until it does; a bot goes by its owner.
//!
//! Where strangers are put in touch by position or school (nearby discovery,
//! bumps, schoolmates) they meet within their own [`AgeGroup`] only, and an
//! account without a birthdate is in neither ([`age_groups_among`]).
//!
//! An account under the minimum age ([`too_young_on`]) cannot use the app
//! until that birthday: its sessions reach signing in and out, `GetMe` and
//! the deletion of the account only
//! (`tokuly_core::auth::reachable_while_too_young`).
use std::collections::{HashMap, HashSet};

use chrono::{Months, NaiveDate, Utc};
use tokuly_core::prelude::*;

/// The age from which the limits for minors no longer apply.
pub const ADULT_AGE: u32 = 18;

/// Under [`ADULT_AGE`] on `today`; no birthdate counts as under.
pub fn minor_on(birthdate: Option<NaiveDate>, today: NaiveDate) -> bool {
    birthdate.is_none_or(|born| today.years_since(born).is_none_or(|age| age < ADULT_AGE))
}

/// Under the minimum age (`min_age`) on `today`. No birthdate is not too
/// young: it counts as a minor ([`minor_on`]).
pub fn too_young_on(birthdate: Option<NaiveDate>, today: NaiveDate, min_age: u32) -> bool {
    birthdate.is_some_and(|born| today.years_since(born).is_none_or(|age| age < min_age))
}

impl crate::Identity {
    /// [`too_young_on`] today, by the minimum age of this deployment. Other
    /// domains ask it for what acts for an account without a session of its
    /// own (API tokens, bots, webhooks).
    pub fn too_young(&self, birthdate: Option<NaiveDate>) -> bool {
        too_young_on(birthdate, Utc::now().date_naive(), self.cfg.min_age)
    }
}

/// The two age groups strangers meet within.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgeGroup {
    Minors,
    Adults,
}

/// Born on this day or before: of age ([`ADULT_AGE`]) on `today`.
pub fn adult_if_born_by(today: NaiveDate) -> NaiveDate {
    today.checked_sub_months(Months::new(12 * ADULT_AGE)).unwrap_or(NaiveDate::MIN)
}

/// The age group of each of `user_ids` today, by the birthdate (a bot by its
/// owner's). Accounts without a birthdate, and unknown IDs, are in neither
/// and left out.
pub async fn age_groups_among(core: &Core, user_ids: &[Uuid]) -> AppResult<HashMap<Uuid, AgeGroup>> {
    if user_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(Uuid, NaiveDate)> = sqlx::query_as(
        "SELECT u.id, CASE WHEN u.bot THEN owner.birthdate ELSE u.birthdate END AS birthdate
         FROM users u LEFT JOIN users owner ON owner.id = u.bot_owner_id
         WHERE u.id = ANY($1) AND CASE WHEN u.bot THEN owner.birthdate ELSE u.birthdate END IS NOT NULL",
    )
    .bind(user_ids)
    .fetch_all(&core.db)
    .await?;
    let adult_by = adult_if_born_by(Utc::now().date_naive());
    Ok(rows
        .into_iter()
        .map(|(id, born)| (id, if born <= adult_by { AgeGroup::Adults } else { AgeGroup::Minors }))
        .collect())
}

/// The age group of `user_id` today; `None` without a birthdate.
pub async fn age_group(core: &Core, user_id: Uuid) -> AppResult<Option<AgeGroup>> {
    Ok(age_groups_among(core, &[user_id]).await?.remove(&user_id))
}

/// Whether the limits for minors apply to `user_id` today. An unknown
/// account counts as a minor.
pub async fn is_minor(core: &Core, user_id: Uuid) -> AppResult<bool> {
    let adults = adults_among(core, &[user_id]).await?;
    Ok(!adults.contains(&user_id))
}

/// The members of `user_ids` the limits for minors apply to today (unknown
/// IDs included).
pub async fn minors_among(core: &Core, user_ids: &[Uuid]) -> AppResult<HashSet<Uuid>> {
    let adults = adults_among(core, user_ids).await?;
    Ok(user_ids.iter().copied().filter(|id| !adults.contains(id)).collect())
}

async fn adults_among(core: &Core, user_ids: &[Uuid]) -> AppResult<HashSet<Uuid>> {
    if user_ids.is_empty() {
        return Ok(HashSet::new());
    }
    let adults: Vec<Uuid> = sqlx::query_scalar(
        "SELECT u.id FROM users u LEFT JOIN users owner ON owner.id = u.bot_owner_id
         WHERE u.id = ANY($1)
           AND CASE WHEN u.bot THEN owner.birthdate ELSE u.birthdate END <= current_date - make_interval(years => $2)",
    )
    .bind(user_ids)
    .bind(ADULT_AGE as i32)
    .fetch_all(&core.db)
    .await?;
    Ok(adults.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn eighteen_on_the_birthday() {
        let born = day(2008, 11, 27);
        assert!(minor_on(Some(born), day(2026, 11, 26)));
        assert!(!minor_on(Some(born), day(2026, 11, 27)));
    }

    #[test]
    fn no_birthdate_counts_as_a_minor() {
        assert!(minor_on(None, day(2026, 11, 27)));
    }

    #[test]
    fn of_age_by_the_same_rule() {
        let today = day(2026, 11, 27);
        assert_eq!(adult_if_born_by(today), day(2008, 11, 27));
        for born in [day(2008, 11, 27), day(2008, 11, 28), day(1990, 1, 1)] {
            assert_eq!(born <= adult_if_born_by(today), !minor_on(Some(born), today), "{born}");
        }
        // Born on the 29th of February: of age on the 1st of March.
        assert!(minor_on(Some(day(2008, 2, 29)), day(2026, 2, 28)));
        assert!(day(2008, 2, 29) > adult_if_born_by(day(2026, 2, 28)));
    }

    #[test]
    fn too_young_until_the_birthday_of_the_minimum_age() {
        let born = day(2013, 11, 28);
        assert!(too_young_on(Some(born), day(2026, 11, 27), 13));
        assert!(!too_young_on(Some(born), day(2026, 11, 28), 13));
        assert!(!too_young_on(None, day(2026, 11, 27), 13), "no birthdate counts as a minor, not as too young");
    }
}
