//! User rows, lookups, summaries and input validation.
//!
//! [`summaries`] is the function other domains call most: it turns user IDs
//! into the `UserSummary` messages embedded all over the API.
use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use tokuly_core::prelude::*;
use tokuly_media::assets::image_from_json;
use tokuly_proto::{
    common::UserSummary,
    identity::{Me, Status},
};
use unicode_segmentation::UnicodeSegmentation as _;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserRow {
    pub id: Uuid,
    pub handle: String,
    pub display_name: String,
    pub email: Option<String>,
    pub email_verified_at: Option<DateTime<Utc>>,
    pub password_hash: Option<String>,
    pub bio: String,
    pub avatar: Option<serde_json::Value>,
    pub avatar_video_url: Option<String>,
    pub banner: Option<serde_json::Value>,
    pub birthdate: Option<NaiveDate>,
    pub locale: String,
    pub time_zone: String,
    pub roles: Vec<String>,
    pub verified: bool,
    pub school_name: String,
    pub status_emoji: String,
    pub status_text: String,
    pub status_set_at: Option<DateTime<Utc>>,
    pub status_expires_at: Option<DateTime<Utc>>,
    pub totp_secret: Option<String>,
    pub totp_enabled_at: Option<DateTime<Utc>>,
    pub privacy: serde_json::Value,
    pub appearance: serde_json::Value,
    pub profile_view_count: i64,
    pub banned_at: Option<DateTime<Utc>>,
    pub deletion_scheduled_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

macro_rules! user_columns {
    () => {
        "id, handle::text AS handle, display_name, email::text AS email, email_verified_at, password_hash, bio, \
         avatar, avatar_video_url, banner, birthdate, locale, time_zone, roles, verified, school_name, \
         status_emoji, status_text, status_set_at, status_expires_at, totp_secret, totp_enabled_at, \
         privacy, appearance, profile_view_count, banned_at, deletion_scheduled_at, deleted_at, created_at"
    };
}
pub(crate) use user_columns;

impl UserRow {
    pub fn is_active(&self) -> bool {
        self.deleted_at.is_none() && self.banned_at.is_none()
    }

    pub fn status(&self) -> Option<Status> {
        status_of(
            &self.status_emoji,
            &self.status_text,
            self.status_set_at,
            self.status_expires_at,
        )
    }

    pub fn summary(&self) -> UserSummary {
        if self.deleted_at.is_some() {
            return deleted_summary(self.id);
        }
        UserSummary {
            id: self.id.to_string(),
            handle: self.handle.clone(),
            display_name: self.display_name.clone(),
            avatar: image_from_json(self.avatar.as_ref()).into(),
            avatar_video_url: self.avatar_video_url.clone().unwrap_or_default(),
            verified: self.verified,
            ..Default::default()
        }
    }

    /// The private view of the account for its owner.
    pub fn to_me(&self, passkey_count: i32, features: HashMap<String, bool>) -> Me {
        Me {
            user: self.summary().into(),
            email: self.email.clone().unwrap_or_default(),
            email_verified: self.email_verified_at.is_some(),
            bio: self.bio.clone(),
            banner: image_from_json(self.banner.as_ref()).into(),
            created_at: ts(self.created_at),
            totp_enabled: self.totp_enabled_at.is_some(),
            passkey_count,
            has_password: self.password_hash.is_some(),
            roles: self.roles.clone(),
            deletion_scheduled_at: opt_ts(self.deletion_scheduled_at),
            status: self.status().into(),
            birthdate: self.birthdate.map(|d| d.to_string()).unwrap_or_default(),
            features: features.into_iter().collect(),
            locale: self.locale.clone(),
            time_zone: self.time_zone.clone(),
            ..Default::default()
        }
    }
}

/// Builds a status message, dropping expired ones.
pub fn status_of(
    emoji: &str,
    text: &str,
    set_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
) -> Option<Status> {
    if emoji.is_empty() && text.is_empty() {
        return None;
    }
    if expires_at.is_some_and(|t| t <= Utc::now()) {
        return None;
    }
    Some(Status {
        emoji: emoji.to_owned(),
        text: text.to_owned(),
        expires_at: opt_ts(expires_at),
        set_at: opt_ts(set_at),
        ..Default::default()
    })
}

fn deleted_summary(id: Uuid) -> UserSummary {
    UserSummary {
        id: id.to_string(),
        handle: "deleted".into(),
        deleted: true,
        ..Default::default()
    }
}

pub async fn find_by_id(core: &Core, id: Uuid) -> AppResult<Option<UserRow>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        user_columns!(),
        " FROM users WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(&core.db)
    .await?)
}

pub async fn get(core: &Core, id: Uuid) -> AppResult<UserRow> {
    find_by_id(core, id)
        .await?
        .or_not_found("USER_NOT_FOUND", "user not found")
}

pub async fn find_by_handle(core: &Core, handle: &str) -> AppResult<Option<UserRow>> {
    let handle = handle.trim().trim_start_matches('@');
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        user_columns!(),
        " FROM users WHERE handle = $1::citext AND deleted_at IS NULL"
    ))
    .bind(handle)
    .fetch_optional(&core.db)
    .await?)
}

/// The one spelling of a sign-in identifier: an address lowercased, a
/// handle without any leading `@` and lowercased. Rate limits key on it, so
/// `@h`, `@@h` and `H` are the same name.
pub fn normalize_identifier(identifier: &str) -> String {
    identifier.trim().trim_start_matches('@').trim().to_ascii_lowercase()
}

/// Looks a user up by email address or handle (sign-in identifier).
pub async fn find_by_identifier(core: &Core, identifier: &str) -> AppResult<Option<UserRow>> {
    let identifier = identifier.trim();
    if identifier.contains('@') && !identifier.starts_with('@') {
        Ok(sqlx::query_as(concat!(
            "SELECT ",
            user_columns!(),
            " FROM users WHERE email = $1::citext AND deleted_at IS NULL"
        ))
        .bind(identifier)
        .fetch_optional(&core.db)
        .await?)
    } else {
        find_by_handle(core, identifier).await
    }
}

#[derive(sqlx::FromRow)]
struct SummaryRow {
    id: Uuid,
    handle: String,
    display_name: String,
    avatar: Option<serde_json::Value>,
    avatar_video_url: Option<String>,
    verified: bool,
    deleted_at: Option<DateTime<Utc>>,
}

/// Summaries for a set of users, keyed by ID. Unknown IDs are simply absent;
/// deleted accounts come back as placeholders with `deleted = true`.
pub async fn summaries(core: &Core, ids: &[Uuid]) -> AppResult<HashMap<Uuid, UserSummary>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<SummaryRow> = sqlx::query_as(
        "SELECT id, handle::text AS handle, display_name, avatar, avatar_video_url, verified, deleted_at FROM users WHERE id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(&core.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let summary = if r.deleted_at.is_some() {
                deleted_summary(r.id)
            } else {
                UserSummary {
                    id: r.id.to_string(),
                    handle: r.handle,
                    display_name: r.display_name,
                    avatar: image_from_json(r.avatar.as_ref()).into(),
                    avatar_video_url: r.avatar_video_url.unwrap_or_default(),
                    verified: r.verified,
                    ..Default::default()
                }
            };
            (r.id, summary)
        })
        .collect())
}

/// Summaries in the same order as `ids`, skipping unknown users.
pub async fn summaries_ordered(core: &Core, ids: &[Uuid]) -> AppResult<Vec<UserSummary>> {
    let map = summaries(core, ids).await?;
    Ok(ids.iter().filter_map(|id| map.get(id).cloned()).collect())
}

/// One summary; a placeholder when the user is unknown.
pub async fn summary(core: &Core, id: Uuid) -> AppResult<UserSummary> {
    Ok(summaries(core, &[id])
        .await?
        .remove(&id)
        .unwrap_or_else(|| deleted_summary(id)))
}

/// Picks a summary out of a prefetched map, with a placeholder fallback.
pub fn pick(map: &HashMap<Uuid, UserSummary>, id: Uuid) -> UserSummary {
    map.get(&id).cloned().unwrap_or_else(|| deleted_summary(id))
}

/// IDs of active (not deleted, not banned) users among `ids`.
pub async fn filter_active(core: &Core, ids: &[Uuid]) -> AppResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar(
        "SELECT id FROM users WHERE id = ANY($1) AND deleted_at IS NULL AND banned_at IS NULL",
    )
    .bind(ids)
    .fetch_all(&core.db)
    .await?)
}

/// Active users whose verified email matches one of `hashes` (hex SHA-256 of
/// the lowercased address), as `(user_id, hash)`. This is how address-book
/// matching finds people without the server ever seeing the address book.
pub async fn find_by_contact_hashes(core: &Core, hashes: &[String]) -> AppResult<Vec<(Uuid, String)>> {
    if hashes.is_empty() {
        return Ok(Vec::new());
    }
    Ok(sqlx::query_as(
        "SELECT id, contact_hash(email) FROM users
         WHERE contact_hash(email) = ANY($1) AND email_verified_at IS NOT NULL AND deleted_at IS NULL AND banned_at IS NULL",
    )
    .bind(hashes)
    .fetch_all(&core.db)
    .await?)
}

/// Other active users who set the same school as `user_id` (empty when the
/// user has no school).
pub async fn schoolmates(core: &Core, user_id: Uuid, limit: i64) -> AppResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar(
        "SELECT mate.id FROM users me JOIN users mate ON mate.school_name = me.school_name AND mate.id <> me.id
         WHERE me.id = $1 AND me.school_name <> '' AND mate.deleted_at IS NULL AND mate.banned_at IS NULL
         ORDER BY mate.id DESC LIMIT $2",
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(&core.db)
    .await?)
}

pub fn validate_school_name(input: &str) -> AppResult<String> {
    let name = input.trim();
    ensure!(name.graphemes(true).count() <= 60, invalid, "SCHOOL_NAME_INVALID", "school name must be at most 60 characters");
    ensure!(!name.chars().any(char::is_control), invalid, "SCHOOL_NAME_INVALID", "school name contains control characters");
    Ok(name.to_owned())
}

// ───────────────────────────── released handles ─────────────────────────────

/// How long a handle that was let go (renamed away from, or freed by an
/// account purge) stays with its previous owner. Links, mentions and
/// screenshots still carry it; whoever took it at once would receive them.
const HANDLE_HOLD: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 3600);

pub(crate) fn held_handle_key(handle: &str) -> String {
    format!("id:handle:held:{}", handle.to_ascii_lowercase())
}

/// Keeps `handle` for `owner` for [`HANDLE_HOLD`].
pub(crate) async fn hold_handle(core: &Core, handle: &str, owner: Uuid) -> AppResult<()> {
    core.cache.set_string(&held_handle_key(handle), &owner.to_string(), Some(HANDLE_HOLD)).await
}

/// Whether `handle` was recently let go by someone other than `claimant`.
pub(crate) async fn handle_held_from(core: &Core, handle: &str, claimant: Option<Uuid>) -> AppResult<bool> {
    let holder = core.cache.get_string(&held_handle_key(handle)).await?;
    Ok(holder.is_some_and(|holder| Some(holder.as_str()) != claimant.map(|c| c.to_string()).as_deref()))
}

// ───────────────────────────── validation ─────────────────────────────

const RESERVED_HANDLES: &[&str] = &[
    "admin",
    "administrator",
    "tokuly",
    "support",
    "help",
    "official",
    "staff",
    "moderator",
    "system",
    "root",
    "api",
    "www",
    "live",
    "studio",
    "shorts",
    "watch",
    "map",
    "chat",
    "notes",
    "news",
    "stamps",
    "settings",
    "login",
    "signup",
    "signin",
    "logout",
    "me",
    "you",
    "null",
    "undefined",
    "deleted",
    "everyone",
    "here",
    "channel",
    "about",
    "terms",
    "privacy",
    "explore",
    "search",
    "invite",
    "embed",
    "media",
    "static",
    "assets",
    "app",
    "download",
];

/// Normalizes a handle and checks the format: 3-24 characters of `a-z`,
/// `0-9`, `_` or `-`, starting with a letter or digit.
pub fn normalize_handle(input: &str) -> Result<String, &'static str> {
    let handle = input.trim().trim_start_matches('@').to_ascii_lowercase();
    let valid = (3..=24).contains(&handle.len())
        && handle
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        && handle.as_bytes()[0].is_ascii_alphanumeric();
    if !valid {
        return Err("INVALID_FORMAT");
    }
    if RESERVED_HANDLES.contains(&handle.as_str()) {
        return Err("RESERVED");
    }
    Ok(handle)
}

/// A handle someone may take as their own (or for a channel, community or
/// bot): [`normalize_handle`] plus no passing for Tokuly or its staff. Any
/// handle that reads "tokuly" once look-alikes are folded (`t0kuly`,
/// `tokuly_official`) is reserved, and so are the reserved words spelled
/// with look-alikes (`adm1n`, `supp0rt`). Lookups of existing handles use
/// [`normalize_handle`] and are not affected.
pub fn claimable_handle(input: &str) -> Result<String, &'static str> {
    let handle = normalize_handle(input)?;
    let folded = fold_lookalikes(&handle);
    if folded.contains("tokuly") || RESERVED_HANDLES.iter().any(|word| fold_lookalikes(word) == folded) {
        return Err("RESERVED");
    }
    Ok(handle)
}

/// A handle as it reads at a glance: separators dropped, digits and letters
/// that pass for one another made the same (`0`/`o`, `1`/`i`/`l`, `rn`/`m`...).
fn fold_lookalikes(handle: &str) -> String {
    let folded: String = handle
        .chars()
        .filter(|c| !matches!(c, '_' | '-'))
        .map(|c| match c {
            '0' => 'o',
            '1' | 'i' => 'l',
            '3' => 'e',
            '4' => 'a',
            '5' => 's',
            '7' => 't',
            '8' => 'b',
            '9' => 'g',
            c => c,
        })
        .collect();
    folded.replace("rn", "m").replace("vv", "w")
}

pub fn validate_handle(input: &str) -> AppResult<String> {
    handle_error(claimable_handle(input))
}

/// The API error for a handle that [`claimable_handle`] refused.
pub(crate) fn handle_error(result: Result<String, &'static str>) -> AppResult<String> {
    result.map_err(|reason| match reason {
        "RESERVED" => AppError::invalid("HANDLE_RESERVED", "this handle is reserved"),
        _ => AppError::invalid(
            "HANDLE_INVALID",
            "handles are 3-24 characters: a-z, 0-9, _ and -",
        ),
    })
}

pub fn validate_display_name(input: &str) -> AppResult<String> {
    let name = input.trim();
    let len = name.graphemes(true).count();
    ensure!(
        (1..=40).contains(&len),
        invalid,
        "DISPLAY_NAME_INVALID",
        "display name must be 1-40 characters"
    );
    ensure!(
        !name.chars().any(char::is_control),
        invalid,
        "DISPLAY_NAME_INVALID",
        "display name contains control characters"
    );
    Ok(name.to_owned())
}

pub fn validate_bio(input: &str) -> AppResult<String> {
    let bio = input.trim();
    ensure!(
        bio.graphemes(true).count() <= 300,
        invalid,
        "BIO_TOO_LONG",
        "bio must be at most 300 characters"
    );
    Ok(bio.to_owned())
}

pub fn validate_email(input: &str) -> AppResult<String> {
    let email = input.trim().to_ascii_lowercase();
    let ok = email.len() <= 254
        && email.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty()
                && local.len() <= 64
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && domain
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
                && local.bytes().all(|b| b.is_ascii_graphic() && b != b'@')
        });
    ensure!(ok, invalid, "EMAIL_INVALID", "enter a valid email address");
    Ok(email)
}

pub fn validate_password(password: &str) -> AppResult<()> {
    let len = password.chars().count();
    ensure!(
        len >= 8,
        invalid,
        "PASSWORD_TOO_SHORT",
        "password must be at least 8 characters"
    );
    ensure!(
        len <= 256,
        invalid,
        "PASSWORD_TOO_LONG",
        "password must be at most 256 characters"
    );
    const WEAK: &[&str] = &[
        "password",
        "12345678",
        "123456789",
        "qwertyui",
        "11111111",
        "00000000",
        "password1",
        "iloveyou",
    ];
    ensure!(
        !WEAK.contains(&password.to_ascii_lowercase().as_str()),
        invalid,
        "PASSWORD_TOO_WEAK",
        "choose a less common password"
    );
    Ok(())
}

/// Parses a birthdate and enforces the minimum age.
pub fn validate_birthdate(input: &str, min_age: u32) -> AppResult<NaiveDate> {
    let date = NaiveDate::parse_from_str(input.trim(), "%Y-%m-%d")
        .map_err(|_| AppError::invalid("BIRTHDATE_INVALID", "birthdate must be YYYY-MM-DD"))?;
    let today = Utc::now().date_naive();
    ensure!(
        date <= today
            && date
                .years_since(NaiveDate::from_ymd_opt(1900, 1, 1).unwrap())
                .is_some(),
        invalid,
        "BIRTHDATE_INVALID",
        "birthdate is out of range"
    );
    let age = today.years_since(date).unwrap_or(0);
    ensure!(
        age >= min_age,
        denied,
        "AGE_RESTRICTED",
        "you must be at least {min_age} years old to use Tokuly"
    );
    Ok(date)
}

pub fn validate_time_zone(input: &str) -> AppResult<String> {
    input
        .parse::<chrono_tz::Tz>()
        .map(|tz| tz.name().to_owned())
        .map_err(|_| AppError::invalid("TIME_ZONE_INVALID", "unknown time zone"))
}

pub fn validate_locale(input: &str) -> AppResult<String> {
    let locale = input.trim();
    let ok = (2..=16).contains(&locale.len())
        && locale
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-');
    ensure!(ok, invalid, "LOCALE_INVALID", "invalid locale");
    Ok(locale.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_normalize_and_reject_bad_input() {
        assert_eq!(normalize_handle("@Toku_Zou-1").unwrap(), "toku_zou-1");
        assert_eq!(normalize_handle("ab"), Err("INVALID_FORMAT"));
        assert_eq!(normalize_handle("_abc"), Err("INVALID_FORMAT"));
        assert_eq!(normalize_handle("日本語です"), Err("INVALID_FORMAT"));
        assert_eq!(normalize_handle("has space"), Err("INVALID_FORMAT"));
        assert_eq!(normalize_handle("Admin"), Err("RESERVED"));
        assert_eq!(normalize_handle(&"a".repeat(25)), Err("INVALID_FORMAT"));
    }

    #[test]
    fn handles_that_pass_for_tokuly_or_staff_are_reserved() {
        for lookalike in [
            "tokuly_official",
            "t0kuly",
            "tokuIy",
            "toku-ly",
            "the_tokuly_team",
            "adm1n",
            "ad_min",
            "supp0rt",
            "m0derator",
            "5taff",
            "0fficial",
        ] {
            assert_eq!(claimable_handle(lookalike), Err("RESERVED"), "{lookalike}");
            assert_eq!(validate_handle(lookalike).unwrap_err().reason, "HANDLE_RESERVED", "{lookalike}");
        }
        for fine in ["toku_zou", "badminton", "staffordshire", "kaito_2009", "admiral"] {
            assert_eq!(claimable_handle(fine).as_deref(), Ok(fine), "{fine}");
        }
        // Lookups of existing handles are not affected.
        assert_eq!(normalize_handle("tokuly_official").as_deref(), Ok("tokuly_official"));
    }

    #[test]
    fn display_names_count_graphemes_not_bytes() {
        assert_eq!(
            validate_display_name("  とくぞう👨‍👩‍👧  ").unwrap(),
            "とくぞう👨‍👩‍👧"
        );
        assert!(validate_display_name("").is_err());
        assert!(validate_display_name(&"あ".repeat(41)).is_err());
        assert!(validate_display_name(&"あ".repeat(40)).is_ok());
    }

    #[test]
    fn emails_are_lowercased_and_checked() {
        assert_eq!(
            validate_email(" Foo@Example.COM ").unwrap(),
            "foo@example.com"
        );
        for bad in [
            "",
            "foo",
            "foo@",
            "@example.com",
            "foo@example",
            "a b@example.com",
            "foo@.com",
        ] {
            assert!(validate_email(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn passwords_need_length_and_some_originality() {
        assert!(validate_password("short").is_err());
        assert!(validate_password("Password").is_err());
        assert!(validate_password("correct horse battery").is_ok());
    }

    #[test]
    fn birthdate_enforces_minimum_age() {
        let today = Utc::now().date_naive();
        let twelve = today.with_year(today.year() - 12).unwrap();
        let twenty = today.with_year(today.year() - 20).unwrap();
        assert_eq!(
            validate_birthdate(&twelve.to_string(), 13)
                .unwrap_err()
                .reason,
            "AGE_RESTRICTED"
        );
        assert!(validate_birthdate(&twenty.to_string(), 13).is_ok());
        assert!(validate_birthdate("not-a-date", 13).is_err());
        assert!(validate_birthdate("2999-01-01", 13).is_err());
    }

    use chrono::Datelike as _;

    #[test]
    fn expired_status_is_hidden() {
        let past = Utc::now() - chrono::Duration::minutes(1);
        assert!(status_of("🍜", "lunch", None, Some(past)).is_none());
        assert!(status_of("🍜", "lunch", None, None).is_some());
        assert!(status_of("", "", None, None).is_none());
    }
}
