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
use std::collections::HashSet;

use chrono::NaiveDate;
use tokuly_core::prelude::*;

/// The age from which the limits for minors no longer apply.
pub const ADULT_AGE: u32 = 18;

/// Under [`ADULT_AGE`] on `today`; no birthdate counts as under.
pub fn minor_on(birthdate: Option<NaiveDate>, today: NaiveDate) -> bool {
    birthdate.is_none_or(|born| today.years_since(born).is_none_or(|age| age < ADULT_AGE))
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
}
