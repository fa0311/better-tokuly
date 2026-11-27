//! TOTP two-factor authentication helpers.
use std::time::Duration;

use tokuly_core::{keys::Keys, prelude::*};
use totp_rs::{Builder, Secret, Totp};

use crate::Identity;

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

pub fn hash_recovery_code(keys: &Keys, code: &str) -> String {
    keys.token_hash(&format!("recovery:{}", normalize_recovery_code(code)))
}

/// What a stored hash of `code` may be: codes made before `TOKULY_SECRET`
/// was replaced work while the previous one is set.
pub fn recovery_code_hashes(keys: &Keys, code: &str) -> Vec<String> {
    keys.token_hashes(&format!("recovery:{}", normalize_recovery_code(code)))
}

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
        assert_eq!(hash_recovery_code(&keys, "AbCd-1234"), hash_recovery_code(&keys, " abcd-1234 "));
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
