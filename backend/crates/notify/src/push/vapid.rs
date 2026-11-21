//! VAPID (RFC 8292): how this server identifies itself to Web Push services.
//!
//! Browsers create a subscription for one application-server key (the public
//! key returned by `GetPushConfig`). Every push request then carries a short
//! ES256 token signed with the matching private key, naming the push service
//! it is meant for.
//!
//! The key pair is derived from the master secret, so every instance signs
//! with the same key without any key file to distribute.
use std::collections::HashMap;

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use p256::{SecretKey, pkcs8::EncodePrivateKey as _};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokuly_core::{
    keys::{Keys, b64url, b64url_decode},
    prelude::*,
};

use super::ece::public_key_bytes;

/// RFC 8292 allows at most 24 hours.
const TOKEN_LIFETIME_SECONDS: i64 = 12 * 3600;
/// A cached token is replaced once it has less than this left.
const REUSE_MARGIN_SECONDS: i64 = 3600;
/// Audiences are push-service origins, so there are only a handful; the bound
/// only matters when any host is allowed.
const MAX_CACHED_TOKENS: usize = 256;

#[derive(Debug, Serialize, Deserialize)]
pub struct VapidClaims {
    /// Origin of the push service the token is for.
    pub aud: String,
    pub exp: i64,
    /// Contact URI of the operator.
    pub sub: String,
}

struct CachedToken {
    token: String,
    expires_at: i64,
}

pub struct Vapid {
    /// Uncompressed public key, base64url: what browsers subscribe with.
    public_key: String,
    signing_key: EncodingKey,
    subject: String,
    /// Tokens by audience. Push services ask senders to reuse tokens rather
    /// than sign one per message.
    tokens: Mutex<HashMap<String, CachedToken>>,
}

impl Vapid {
    pub fn new(secret: &SecretKey, subject: String) -> anyhow::Result<Self> {
        anyhow::ensure!(
            subject.starts_with("mailto:") || subject.starts_with("https://"),
            "the VAPID subject must be a mailto: or https: URI (TOKULY_VAPID_SUBJECT), got {subject:?}"
        );
        let pkcs8 = secret
            .to_pkcs8_der()
            .map_err(|e| anyhow::anyhow!("cannot encode the VAPID key: {e}"))?;
        Ok(Self {
            public_key: b64url(&public_key_bytes(secret)),
            signing_key: EncodingKey::from_ec_der(pkcs8.as_bytes()),
            subject,
            tokens: Mutex::new(HashMap::new()),
        })
    }

    pub fn public_key(&self) -> &str {
        &self.public_key
    }

    /// The `Authorization` header value for a request to `endpoint`.
    pub fn authorization(&self, endpoint: &url::Url) -> AppResult<String> {
        let audience = endpoint.origin().ascii_serialization();
        let now = chrono::Utc::now().timestamp();
        let mut tokens = self.tokens.lock();
        if let Some(cached) = tokens.get(&audience)
            && cached.expires_at - now > REUSE_MARGIN_SECONDS
        {
            return Ok(self.header_value(&cached.token));
        }
        let expires_at = now + TOKEN_LIFETIME_SECONDS;
        let claims = VapidClaims {
            aud: audience.clone(),
            exp: expires_at,
            sub: self.subject.clone(),
        };
        let token =
            jsonwebtoken::encode(&Header::new(Algorithm::ES256), &claims, &self.signing_key)
                .map_err(|e| {
                    AppError::internal(anyhow::anyhow!("VAPID token signing failed: {e}"))
                })?;
        if tokens.len() >= MAX_CACHED_TOKENS {
            tokens.clear();
        }
        let value = self.header_value(&token);
        tokens.insert(audience, CachedToken { token, expires_at });
        Ok(value)
    }

    fn header_value(&self, token: &str) -> String {
        format!("vapid t={token}, k={}", self.public_key)
    }
}

/// The key every instance derives from the master secret.
pub fn derive_secret(keys: &Keys) -> SecretKey {
    // 32 derived bytes are a valid P-256 scalar except with probability about
    // 2^-32; the attempt counter makes the derivation total and deterministic.
    (0u32..)
        .find_map(|attempt| {
            let label = if attempt == 0 {
                "webpush/vapid".to_owned()
            } else {
                format!("webpush/vapid/{attempt}")
            };
            SecretKey::from_slice(&keys.derive(&label)).ok()
        })
        .expect("some derived value is a valid scalar")
}

/// Parses a configured private key: the raw 32-byte scalar in base64url, the
/// format `web-push generate-vapid-keys` prints.
pub fn parse_secret(encoded: &str) -> anyhow::Result<SecretKey> {
    let bytes = b64url_decode(encoded.trim().trim_end_matches('='))
        .ok_or_else(|| anyhow::anyhow!("TOKULY_VAPID_PRIVATE_KEY is not base64url"))?;
    SecretKey::from_slice(&bytes).map_err(|_| {
        anyhow::anyhow!(
            "TOKULY_VAPID_PRIVATE_KEY is not a P-256 private key (expected 32 bytes, base64url)"
        )
    })
}

/// The contact URI to present: the configured one, else the public URL when
/// it is https (push services refuse other schemes), else the mail sender.
pub fn subject_from(configured: &str, public_url: &str, mail_from: &str) -> String {
    let configured = configured.trim();
    if !configured.is_empty() {
        return configured.to_owned();
    }
    if public_url.starts_with("https://") {
        return public_url.to_owned();
    }
    // `Tokuly <no-reply@example.com>` → `no-reply@example.com`
    let address = match (mail_from.find('<'), mail_from.rfind('>')) {
        (Some(start), Some(end)) if start < end => &mail_from[start + 1..end],
        _ => mail_from,
    };
    format!("mailto:{}", address.trim())
}

#[cfg(test)]
mod tests {
    use jsonwebtoken::{DecodingKey, Validation};

    use super::*;

    fn keys() -> Keys {
        Keys::new("0123456789abcdef0123456789abcdef")
    }

    fn vapid() -> Vapid {
        Vapid::new(&derive_secret(&keys()), "mailto:ops@tokuly.example".into()).unwrap()
    }

    /// Verifies a token against the advertised public key, as a push service would.
    fn verify(vapid: &Vapid, token: &str, audience: &str) -> VapidClaims {
        let public = b64url_decode(vapid.public_key()).unwrap();
        assert_eq!(
            (public.len(), public[0]),
            (65, 0x04),
            "uncompressed P-256 point"
        );
        let key = DecodingKey::from_ec_components(&b64url(&public[1..33]), &b64url(&public[33..]))
            .unwrap();
        let mut validation = Validation::new(Algorithm::ES256);
        validation.set_audience(&[audience]);
        validation.set_required_spec_claims(&["exp", "aud", "sub"]);
        jsonwebtoken::decode::<VapidClaims>(token, &key, &validation)
            .expect("token verifies with the advertised key")
            .claims
    }

    fn token_of(header: &str) -> &str {
        header
            .strip_prefix("vapid t=")
            .unwrap()
            .split(", k=")
            .next()
            .unwrap()
    }

    #[test]
    fn key_derivation_is_deterministic_per_secret() {
        let first = derive_secret(&keys());
        assert_eq!(first.to_bytes(), derive_secret(&keys()).to_bytes());
        let other = derive_secret(&Keys::new("another-secret-another-secret-123"));
        assert_ne!(first.to_bytes(), other.to_bytes());
    }

    #[test]
    fn authorization_header_carries_a_verifiable_token() {
        let vapid = vapid();
        let endpoint = url::Url::parse("https://fcm.googleapis.com/fcm/send/abc123").unwrap();
        let header = vapid.authorization(&endpoint).unwrap();
        assert!(header.ends_with(&format!(", k={}", vapid.public_key())));
        let claims = verify(&vapid, token_of(&header), "https://fcm.googleapis.com");
        assert_eq!(claims.sub, "mailto:ops@tokuly.example");
        let lifetime = claims.exp - chrono::Utc::now().timestamp();
        assert!(
            lifetime > 0 && lifetime <= 24 * 3600,
            "RFC 8292 caps the lifetime at 24 hours"
        );
    }

    #[test]
    fn tokens_are_reused_per_push_service() {
        let vapid = vapid();
        let chrome = url::Url::parse("https://fcm.googleapis.com/fcm/send/one").unwrap();
        let chrome_other = url::Url::parse("https://fcm.googleapis.com/fcm/send/two").unwrap();
        let firefox =
            url::Url::parse("https://updates.push.services.mozilla.com/wpush/v2/x").unwrap();
        let first = vapid.authorization(&chrome).unwrap();
        assert_eq!(
            first,
            vapid.authorization(&chrome_other).unwrap(),
            "same audience, same token"
        );
        let other = vapid.authorization(&firefox).unwrap();
        assert_ne!(first, other);
        verify(
            &vapid,
            token_of(&other),
            "https://updates.push.services.mozilla.com",
        );
    }

    #[test]
    fn audience_keeps_a_non_default_port() {
        let vapid = vapid();
        let endpoint = url::Url::parse("http://127.0.0.1:9911/push/x").unwrap();
        let header = vapid.authorization(&endpoint).unwrap();
        verify(&vapid, token_of(&header), "http://127.0.0.1:9911");
    }

    #[test]
    fn configured_keys_are_parsed_and_bad_ones_rejected() {
        let secret = derive_secret(&keys());
        let encoded = b64url(&secret.to_bytes());
        assert_eq!(
            parse_secret(&encoded).unwrap().to_bytes(),
            secret.to_bytes()
        );
        assert_eq!(
            parse_secret(&format!(" {encoded}= ")).unwrap().to_bytes(),
            secret.to_bytes(),
            "padding and whitespace are tolerated"
        );
        assert!(parse_secret("not base64 !!").is_err());
        assert!(parse_secret(&b64url(&[1u8; 16])).is_err(), "wrong length");
    }

    #[test]
    fn subject_falls_back_to_public_url_then_mail_sender() {
        assert_eq!(
            subject_from("mailto:a@b.c", "https://x.example", "T <t@x.example>"),
            "mailto:a@b.c"
        );
        assert_eq!(
            subject_from("", "https://x.example", "T <t@x.example>"),
            "https://x.example"
        );
        assert_eq!(
            subject_from(
                "",
                "http://localhost:8080",
                "Tokuly <no-reply@tokuly.localhost>"
            ),
            "mailto:no-reply@tokuly.localhost"
        );
        assert_eq!(
            subject_from("", "http://localhost:8080", "plain@tokuly.localhost"),
            "mailto:plain@tokuly.localhost"
        );
        assert!(Vapid::new(&derive_secret(&keys()), "ftp://nope".into()).is_err());
    }
}
