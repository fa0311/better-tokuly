//! TOTP two-factor authentication helpers.
use std::time::Duration;

use tokuly_core::{events::Event, keys::Keys, prelude::*};
use totp_rs::{Builder, Secret, Totp};

use crate::{Identity, maintenance::UserRef};

/// The topic of [`Identity::forget_recovery_codes`]: `{ user_id }` of an
/// account whose recovery codes stopped working.
pub const RECOVERY_CODES_REVOKED: &str = "identity.user.recovery_codes_revoked";

/// How long a spent time step is remembered: a code is accepted from one
/// step before its own until one step after, so 90 s at most.
const SPENT_STEP_TTL: Duration = Duration::from_secs(120);

fn build(secret_base32: &str, account: &str) -> AppResult<Totp> {
    let secret =
        Secret::try_from_base32(secret_base32).map_err(|e| AppError::internal(anyhow::anyhow!("totp secret: {e}")))?;
    Builder::new()
        .with_secret(secret)
        .with_account_name(account.to_owned())
        .with_issuer(Some("Tokuly"))
        .build()
        .map_err(|e| AppError::internal(anyhow::anyhow!("totp: {e}")))
}

/// A new random secret (base32) and its `otpauth://` URL for QR codes.
pub fn generate(account: &str) -> AppResult<(String, String)> {
    let secret = Secret::generate().to_base32();
    let url = build(&secret, account)?.to_url().map_err(|e| AppError::internal(anyhow::anyhow!("totp url: {e}")))?;
    Ok((secret, url))
}

/// Checks a 6-digit code against the secret (allowing one step of clock
/// skew); returns the time step it belongs to. Use
/// [`Identity::accept_totp`], which also makes sure a code works only once.
pub fn check(secret_base32: &str, code: &str) -> Option<u64> {
    let code: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
    if code.len() != 6 {
        return None;
    }
    build(secret_base32, "account").ok()?.check_current(&code)
}

impl Identity {
    /// Counts one attempt at a code (or recovery code) for `user_id`; fails
    /// with `RATE_LIMITED` past 8 per 5 minutes or 30 per day. The burst
    /// limit alone would allow ~2300 guesses a day, and with three codes
    /// valid at any moment someone who has the password would get in within
    /// a month about one time in five.
    pub(crate) async fn limit_totp_attempts(&self, user_id: Uuid) -> AppResult<()> {
        let cache = &self.core.cache;
        cache.rate_limit(&format!("totp:{user_id}"), 8, Duration::from_secs(300)).await?;
        cache.rate_limit(&format!("totp-day:{user_id}"), 30, Duration::from_secs(24 * 3600)).await
    }

    /// Whether `code` is a valid, unspent TOTP code for `secret`, spending it
    /// if so: RFC 6238 §5.2 — the same code is never accepted twice, so one
    /// that was seen (over a shoulder, through a phishing proxy) is worthless.
    pub(crate) async fn accept_totp(&self, user_id: Uuid, secret: &str, code: &str) -> AppResult<bool> {
        let Some(step) = check(secret, code) else {
            return Ok(false);
        };
        self.core.cache.set_nx(&format!("id:totp:spent:{user_id}:{step}"), "1", SPENT_STEP_TTL).await
    }
}

/// Ten single-use recovery codes like `k3f9-x2mq`.
pub fn generate_recovery_codes() -> Vec<String> {
    (0..10)
        .map(|_| {
            let raw = tokuly_core::ids::random_base62(8).to_ascii_lowercase();
            format!("{}-{}", &raw[..4], &raw[4..])
        })
        .collect()
}

pub fn normalize_recovery_code(code: &str) -> String {
    code.trim().to_ascii_lowercase()
}

/// The label `users.recovery_codes` is sealed with.
pub const RECOVERY_CODES_LABEL: &str = "totp/recovery";

/// An account's unused recovery codes, sealed together for
/// `users.recovery_codes` (`None` once none is left). Sealed rather than
/// hashed so that `tokuly rekey` carries them to a new `TOKULY_SECRET`.
pub fn seal_recovery_codes(keys: &Keys, codes: &[String]) -> Option<String> {
    (!codes.is_empty())
        .then(|| tokuly_core::keys::b64url(&keys.seal(RECOVERY_CODES_LABEL, codes.join("\n").as_bytes())))
}

pub fn open_recovery_codes(keys: &Keys, stored: &str) -> Option<Vec<String>> {
    let sealed = tokuly_core::keys::b64url_decode(stored)?;
    let plain = String::from_utf8(keys.open(RECOVERY_CODES_LABEL, &sealed)?).ok()?;
    Some(plain.split('\n').map(str::to_owned).collect())
}

/// The codes left once `code` is spent, or `None` when it is not one of
/// them. Every code is compared, in constant time, whatever matches.
fn spend_recovery_code(codes: &[String], code: &str) -> Option<Vec<String>> {
    use subtle::ConstantTimeEq as _;
    let code = normalize_recovery_code(code);
    let hits: Vec<bool> = codes.iter().map(|known| bool::from(known.as_bytes().ct_eq(code.as_bytes()))).collect();
    hits.contains(&true)
        .then(|| codes.iter().zip(hits).filter(|(_, hit)| !hit).map(|(known, _)| known.clone()).collect())
}

/// What a stored hash of `code` may be, for codes made before they were
/// sealed (`users.recovery_code_hashes`): they work while the secret they
/// were made under is set.
pub fn recovery_code_hashes(keys: &Keys, code: &str) -> Vec<String> {
    keys.token_hashes(&format!("recovery:{}", normalize_recovery_code(code)))
}

impl Identity {
    /// Spends `code` if it is one of `user_id`'s unused recovery codes.
    pub(crate) async fn spend_recovery_code(&self, user_id: Uuid, code: &str) -> AppResult<bool> {
        if self.spend_sealed_recovery_code(user_id, code).await? {
            return Ok(true);
        }
        let spent: Option<Uuid> = sqlx::query_scalar(
            "UPDATE users SET recovery_code_hashes = ARRAY(SELECT h FROM unnest(recovery_code_hashes) h WHERE h <> ALL($2))
             WHERE id = $1 AND recovery_code_hashes && $2 RETURNING id",
        )
        .bind(user_id)
        .bind(recovery_code_hashes(&self.core.keys, code))
        .fetch_optional(&self.core.db)
        .await?;
        Ok(spent.is_some())
    }

    /// The column is written back only if it still holds what was read: a
    /// code spent at the same moment, or `tokuly rekey` sealing them again,
    /// makes it read them once more.
    async fn spend_sealed_recovery_code(&self, user_id: Uuid, code: &str) -> AppResult<bool> {
        for _ in 0..SPEND_ATTEMPTS {
            let stored: Option<String> = sqlx::query_scalar("SELECT recovery_codes FROM users WHERE id = $1")
                .bind(user_id)
                .fetch_optional(&self.core.db)
                .await?
                .flatten();
            let Some(left) = stored
                .as_deref()
                .and_then(|stored| open_recovery_codes(&self.core.keys, stored))
                .and_then(|codes| spend_recovery_code(&codes, code))
            else {
                return Ok(false);
            };
            let written = sqlx::query("UPDATE users SET recovery_codes = $2 WHERE id = $1 AND recovery_codes = $3")
                .bind(user_id)
                .bind(seal_recovery_codes(&self.core.keys, &left))
                .bind(&stored)
                .execute(&self.core.db)
                .await?;
            if written.rows_affected() == 1 {
                return Ok(true);
            }
        }
        Err(AppError::internal(anyhow::anyhow!("recovery codes of {user_id} kept changing")))
    }

    /// `tokuly rekey --forget-recovery-codes`, for a secret that leaked:
    /// with it, a copy of the database opens every recovery code. All of
    /// them stop working, each account with a note in its security log and
    /// [`RECOVERY_CODES_REVOKED`] on the outbox (the mail of
    /// [`Identity::tell_recovery_codes_revoked`]); returns how many accounts
    /// had some.
    pub async fn forget_recovery_codes(&self) -> anyhow::Result<u64> {
        let forgotten: i64 = sqlx::query_scalar(
            "WITH forgotten AS (
                 UPDATE users SET recovery_codes = NULL, recovery_code_hashes = '{}'
                 WHERE recovery_codes IS NOT NULL OR cardinality(recovery_code_hashes) > 0 RETURNING id
             ), logged AS (
                 INSERT INTO security_events (user_id, type) SELECT id, 'RECOVERY_CODES_REVOKED' FROM forgotten
             ), told AS (
                 INSERT INTO outbox (topic, data) SELECT $1, jsonb_build_object('user_id', id) FROM forgotten
             )
             SELECT count(*) FROM forgotten",
        )
        .bind(RECOVERY_CODES_REVOKED)
        .fetch_one(&self.core.db)
        .await?;
        Ok(forgotten as u64)
    }

    /// [`RECOVERY_CODES_REVOKED`]: mails the account that its recovery
    /// codes stopped working and how to get new ones (it may be locked out,
    /// and then cannot read its security log). Once per event, however
    /// often it arrives ([`Identity::mail_once`]); `Ok(false)` when nothing
    /// was sent: sent before, or the account has no address any more.
    pub async fn tell_recovery_codes_revoked(&self, event: &Event) -> AppResult<bool> {
        let user = crate::users::find_by_id(&self.core, event.data::<UserRef>()?.user_id).await?;
        let Some((email, locale)) = user.and_then(|user| Some((user.email?, user.locale))) else { return Ok(false) };
        self.mail_once(&event.id.to_string(), self.send_recovery_codes_revoked_mail(&email, &locale)).await
    }
}

/// Reads of `users.recovery_codes` before a spend gives up (each lost to a write at the same moment).
const SPEND_ATTEMPTS: usize = 5;

pub fn seal_secret(keys: &Keys, secret_base32: &str) -> String {
    tokuly_core::keys::b64url(&keys.seal("totp", secret_base32.as_bytes()))
}

pub fn open_secret(keys: &Keys, stored: &str) -> Option<String> {
    tokuly_core::keys::b64url_decode(stored)
        .and_then(|sealed| keys.open("totp", &sealed))
        .and_then(|b| String::from_utf8(b).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_secret_validates_its_own_codes() {
        let (secret, url) = generate("toku@example.com").unwrap();
        assert!(url.starts_with("otpauth://totp/") && url.contains("issuer=Tokuly"));
        let code = build(&secret, "x").unwrap().generate_current().to_string();
        assert!(check(&secret, &code).is_some());
        assert!(check(&secret, &format!("{} {}", &code[..3], &code[3..])).is_some(), "spaces are tolerated");
        assert!(check(&secret, "abcdef").is_none());
        let wrong = if code == "000000" { "111111" } else { "000000" };
        assert!(check(&secret, wrong).is_none());
    }

    #[test]
    fn recovery_codes_are_unique_and_hash_case_insensitively() {
        let keys = Keys::new("0123456789abcdef0123456789abcdef");
        let codes = generate_recovery_codes();
        assert_eq!(codes.len(), 10);
        assert!(codes.iter().all(|c| c.len() == 9 && c.as_bytes()[4] == b'-'));
        let unique: std::collections::HashSet<_> = codes.iter().collect();
        assert_eq!(unique.len(), 10);
        assert_eq!(recovery_code_hashes(&keys, "AbCd-1234"), recovery_code_hashes(&keys, " abcd-1234 "));
    }

    #[test]
    fn sealed_recovery_codes_are_spent_once_whatever_the_case() {
        let keys = Keys::new("0123456789abcdef0123456789abcdef");
        let codes = generate_recovery_codes();
        let sealed = seal_recovery_codes(&keys, &codes).unwrap();
        assert!(codes.iter().all(|code| !sealed.contains(code.as_str())));
        let opened = open_recovery_codes(&keys, &sealed).unwrap();
        assert_eq!(opened, codes);
        let left = spend_recovery_code(&opened, &format!(" {} ", codes[3].to_uppercase())).unwrap();
        assert_eq!(left.len(), 9);
        assert!(!left.contains(&codes[3]));
        assert!(spend_recovery_code(&left, &codes[3]).is_none(), "spent");
        assert!(spend_recovery_code(&left, "").is_none());
        assert_eq!(seal_recovery_codes(&keys, &[]), None, "none left");
        assert!(open_recovery_codes(&Keys::new("another-secret-another-secret-another"), &sealed).is_none());
    }

    #[test]
    fn sealed_secret_roundtrips() {
        let keys = Keys::new("0123456789abcdef0123456789abcdef");
        let sealed = seal_secret(&keys, "JBSWY3DPEHPK3PXP");
        assert_ne!(sealed, "JBSWY3DPEHPK3PXP");
        assert_eq!(open_secret(&keys, &sealed).as_deref(), Some("JBSWY3DPEHPK3PXP"));
        assert!(open_secret(&keys, "garbage").is_none());
    }
}
