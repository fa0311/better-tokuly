//! Push registrations: which devices of a user receive push notifications.
//!
//! A registration belongs to the session that created it. It is removed when
//! the client unregisters, when the push service says the token is gone, and
//! when its session ends: a revoked or expired session's registrations are
//! skipped (and deleted) by the delivery job, and swept hourly. Identity does
//! not announce session revocations as events, so checking `sessions` at the
//! moment of sending is what guarantees a signed-out device stays silent.
use sha2::{Digest as _, Sha256};
use tokuly_core::{keys::b64url_decode, prelude::*};
use tokuly_proto::{
    common::Platform,
    notify::{PushChannelKind, PushDevice, RegisterDeviceRequest, register_device_request::Token},
};

use crate::push::{
    PushTarget, ece,
    webpush::{EndpointError, EndpointPolicy},
};

const MAX_TOKEN_LEN: usize = 4096;
const MAX_DEVICE_NAME_CHARS: usize = 100;

/// A validated registration request.
#[derive(Debug)]
pub struct NewDevice {
    pub channel: PushChannelKind,
    pub platform: Platform,
    pub token: String,
    pub p256dh: Option<Vec<u8>>,
    pub auth: Option<Vec<u8>>,
    pub apns_sandbox: bool,
    pub locale: String,
    pub device_name: String,
}

/// Checks a registration request and normalizes its token.
pub fn validate(policy: &EndpointPolicy, request: RegisterDeviceRequest) -> AppResult<NewDevice> {
    let locale = if request.locale.trim().is_empty() {
        String::new()
    } else {
        tokuly_identity::users::validate_locale(&request.locale)?
    };
    let device_name = request.device_name.trim().to_owned();
    ensure!(
        device_name.chars().count() <= MAX_DEVICE_NAME_CHARS,
        invalid,
        "DEVICE_NAME_TOO_LONG",
        "device names have at most {MAX_DEVICE_NAME_CHARS} characters"
    );
    let platform = request
        .platform
        .as_known()
        .unwrap_or(Platform::PLATFORM_UNSPECIFIED);
    let or_default = |default: Platform| {
        if platform == Platform::PLATFORM_UNSPECIFIED {
            default
        } else {
            platform
        }
    };

    let Some(token) = request.token else {
        bail!(
            invalid,
            "TOKEN_REQUIRED",
            "a push subscription or device token is required"
        );
    };
    Ok(match token {
        Token::WebPush(subscription) => {
            match policy.check(&subscription.endpoint) {
                Ok(_) => {}
                Err(EndpointError::Invalid) => bail!(
                    invalid,
                    "ENDPOINT_INVALID",
                    "the push endpoint is not a valid https URL"
                ),
                Err(EndpointError::NotAllowed) => bail!(
                    invalid,
                    "ENDPOINT_NOT_ALLOWED",
                    "the push endpoint does not belong to a known push service"
                ),
            }
            let p256dh = decode_key(&subscription.p256dh)
                .filter(|key| ece::validate_public_key(key).is_ok());
            let auth = decode_key(&subscription.auth)
                .filter(|secret| secret.len() == ece::AUTH_SECRET_LEN);
            let (Some(p256dh), Some(auth)) = (p256dh, auth) else {
                bail!(
                    invalid,
                    "SUBSCRIPTION_KEYS_INVALID",
                    "p256dh must be a P-256 public key and auth a 16-byte secret, both base64url"
                );
            };
            NewDevice {
                channel: PushChannelKind::PUSH_CHANNEL_KIND_WEB_PUSH,
                platform: or_default(Platform::PLATFORM_WEB),
                token: subscription.endpoint.clone(),
                p256dh: Some(p256dh),
                auth: Some(auth),
                apns_sandbox: false,
                locale,
                device_name,
            }
        }
        Token::FcmToken(token) => {
            let token = token.trim().to_owned();
            let printable = token.bytes().all(|b| b.is_ascii_graphic());
            ensure!(
                !token.is_empty() && token.len() <= MAX_TOKEN_LEN && printable,
                invalid,
                "TOKEN_INVALID",
                "not an FCM registration token"
            );
            NewDevice {
                channel: PushChannelKind::PUSH_CHANNEL_KIND_FCM,
                platform: or_default(Platform::PLATFORM_ANDROID),
                token,
                p256dh: None,
                auth: None,
                apns_sandbox: false,
                locale,
                device_name,
            }
        }
        Token::Apns(apns) => {
            let token = apns.token.trim().to_ascii_lowercase();
            // 32 bytes today; Apple reserves the right to make tokens longer.
            let hex = token.len() >= 64
                && token.len() <= 400
                && token.len() % 2 == 0
                && token.bytes().all(|b| b.is_ascii_hexdigit());
            ensure!(
                hex,
                invalid,
                "TOKEN_INVALID",
                "an APNs device token is at least 32 bytes as hex"
            );
            NewDevice {
                channel: PushChannelKind::PUSH_CHANNEL_KIND_APNS,
                platform: or_default(Platform::PLATFORM_IOS),
                token,
                p256dh: None,
                auth: None,
                apns_sandbox: apns.sandbox,
                locale,
                device_name,
            }
        }
    })
}

/// Decodes a subscription key. Browsers produce unpadded base64url; some
/// client libraries pad it or use the standard alphabet.
fn decode_key(encoded: &str) -> Option<Vec<u8>> {
    let normalized = encoded
        .trim()
        .trim_end_matches('=')
        .replace('+', "-")
        .replace('/', "_");
    b64url_decode(&normalized)
}

#[derive(Debug, sqlx::FromRow)]
pub struct DeviceRow {
    pub id: Uuid,
    pub session_id: Uuid,
    pub channel: i32,
    pub platform: i32,
    pub locale: String,
    pub device_name: String,
    pub created_at: Time,
    pub last_delivered_at: Option<Time>,
}

/// Columns of [`DeviceRow`]; queries alias `push_devices` as `d`.
macro_rules! device_columns {
    () => {
        "d.id, d.session_id, d.channel, d.platform, d.locale, d.device_name, d.created_at, d.last_delivered_at"
    };
}

impl DeviceRow {
    pub fn to_proto(&self, current_session: Uuid) -> PushDevice {
        PushDevice {
            id: self.id.to_string(),
            channel: self.channel.into(),
            platform: self.platform.into(),
            locale: self.locale.clone(),
            device_name: self.device_name.clone(),
            current: self.session_id == current_session,
            created_at: ts(self.created_at),
            last_delivered_at: opt_ts(self.last_delivered_at),
            ..Default::default()
        }
    }
}

/// Stores a registration for `session_id`. A session holds one token per
/// channel, and a token belongs to one session: the token it replaces is
/// dropped, and a token another session (or account) registered earlier —
/// the same browser after switching accounts — moves here.
pub async fn register(
    core: &Core,
    user_id: Uuid,
    session_id: Uuid,
    device: &NewDevice,
) -> AppResult<DeviceRow> {
    let channel = device.channel as i32;
    let mut tx = core.db.begin().await?;
    sqlx::query("DELETE FROM push_devices WHERE session_id = $1 AND channel = $2 AND token <> $3")
        .bind(session_id)
        .bind(channel)
        .bind(&device.token)
        .execute(&mut *tx)
        .await?;
    let row: DeviceRow = sqlx::query_as(concat!(
        "INSERT INTO push_devices AS d (user_id, session_id, channel, platform, token, token_hash, p256dh, auth, apns_sandbox, locale, device_name)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
         ON CONFLICT (channel, token_hash) DO UPDATE SET
            user_id = EXCLUDED.user_id, session_id = EXCLUDED.session_id, platform = EXCLUDED.platform,
            p256dh = EXCLUDED.p256dh, auth = EXCLUDED.auth, apns_sandbox = EXCLUDED.apns_sandbox,
            locale = EXCLUDED.locale, device_name = EXCLUDED.device_name,
            rejections = 0, last_error = ''
         RETURNING ",
        device_columns!()
    ))
    .bind(user_id)
    .bind(session_id)
    .bind(channel)
    .bind(device.platform as i32)
    .bind(&device.token)
    .bind(Sha256::digest(device.token.as_bytes()).as_slice())
    .bind(&device.p256dh)
    .bind(&device.auth)
    .bind(device.apns_sandbox)
    .bind(&device.locale)
    .bind(&device.device_name)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// A user's registrations whose session is still alive, newest first.
pub async fn list(core: &Core, user_id: Uuid) -> AppResult<Vec<DeviceRow>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        device_columns!(),
        " FROM push_devices d JOIN sessions s ON s.id = d.session_id
         WHERE d.user_id = $1 AND s.revoked_at IS NULL AND s.expires_at > now()
         ORDER BY d.created_at DESC, d.id DESC"
    ))
    .bind(user_id)
    .fetch_all(&core.db)
    .await?)
}

/// A registration as the delivery job needs it.
#[derive(Debug, sqlx::FromRow)]
pub struct DeliveryDevice {
    pub id: Uuid,
    pub channel: i32,
    pub token: String,
    pub p256dh: Option<Vec<u8>>,
    pub auth: Option<Vec<u8>>,
    pub apns_sandbox: bool,
    pub locale: String,
    /// The session was revoked or expired: do not deliver, drop the row.
    pub session_ended: bool,
}

impl DeliveryDevice {
    pub fn target(&self) -> PushTarget {
        PushTarget {
            token: self.token.clone(),
            p256dh: self.p256dh.clone(),
            auth: self.auth.clone(),
            apns_sandbox: self.apns_sandbox,
        }
    }
}

/// Every registration of a user, each flagged with whether its session is
/// still alive.
pub async fn for_delivery(core: &Core, user_id: Uuid) -> AppResult<Vec<DeliveryDevice>> {
    Ok(sqlx::query_as(
        "SELECT d.id, d.channel, d.token, d.p256dh, d.auth, d.apns_sandbox, d.locale,
                (s.revoked_at IS NOT NULL OR s.expires_at <= now()) AS session_ended
         FROM push_devices d JOIN sessions s ON s.id = d.session_id
         WHERE d.user_id = $1
         ORDER BY d.id",
    )
    .bind(user_id)
    .fetch_all(&core.db)
    .await?)
}

pub async fn delete(core: &Core, device_id: Uuid) -> AppResult<()> {
    sqlx::query("DELETE FROM push_devices WHERE id = $1")
        .bind(device_id)
        .execute(&core.db)
        .await?;
    Ok(())
}

/// Removes registrations whose session was revoked or expired. Returns how many.
pub async fn sweep_ended_sessions(core: &Core) -> AppResult<u64> {
    let done = sqlx::query("DELETE FROM push_devices d USING sessions s WHERE s.id = d.session_id AND (s.revoked_at IS NOT NULL OR s.expires_at <= now())")
        .execute(&core.db)
        .await?;
    Ok(done.rows_affected())
}

#[cfg(test)]
mod tests {
    use tokuly_core::keys::b64url;
    use tokuly_proto::notify::{ApnsToken, WebPushSubscription};

    use super::*;
    use crate::config::NotifyConfig;

    fn policy() -> EndpointPolicy {
        EndpointPolicy::from_config(&NotifyConfig::default())
    }

    fn web_push(endpoint: &str, p256dh: &str, auth: &str) -> RegisterDeviceRequest {
        RegisterDeviceRequest {
            token: Some(Token::from(WebPushSubscription {
                endpoint: endpoint.into(),
                p256dh: p256dh.into(),
                auth: auth.into(),
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    fn valid_keys() -> (String, String) {
        (
            b64url(&ece::public_key_bytes(&ece::random_secret_key())),
            b64url(&[9u8; 16]),
        )
    }

    fn reason(request: RegisterDeviceRequest) -> String {
        validate(&policy(), request)
            .unwrap_err()
            .reason
            .into_owned()
    }

    #[test]
    fn web_push_subscriptions_are_validated() {
        let (p256dh, auth) = valid_keys();
        let endpoint = "https://fcm.googleapis.com/fcm/send/abc";
        let device = validate(&policy(), web_push(endpoint, &p256dh, &auth)).unwrap();
        assert_eq!(device.channel, PushChannelKind::PUSH_CHANNEL_KIND_WEB_PUSH);
        assert_eq!(
            device.platform,
            Platform::PLATFORM_WEB,
            "the platform defaults by channel"
        );
        assert_eq!(
            (device.token.as_str(), device.auth.as_deref()),
            (endpoint, Some(&[9u8; 16][..]))
        );

        assert_eq!(reason(web_push("nope", &p256dh, &auth)), "ENDPOINT_INVALID");
        assert_eq!(
            reason(web_push("https://attacker.example/x", &p256dh, &auth)),
            "ENDPOINT_NOT_ALLOWED"
        );
        assert_eq!(
            reason(web_push(endpoint, "AAAA", &auth)),
            "SUBSCRIPTION_KEYS_INVALID"
        );
        assert_eq!(
            reason(web_push(endpoint, &b64url(&[4u8; 65]), &auth)),
            "SUBSCRIPTION_KEYS_INVALID",
            "not a curve point"
        );
        assert_eq!(
            reason(web_push(endpoint, &p256dh, &b64url(&[1u8; 8]))),
            "SUBSCRIPTION_KEYS_INVALID"
        );
        assert_eq!(
            reason(web_push(endpoint, &p256dh, "***")),
            "SUBSCRIPTION_KEYS_INVALID"
        );
    }

    #[test]
    fn padded_and_standard_base64_keys_are_accepted() {
        use base64::Engine as _;
        let secret = ece::random_secret_key();
        let p256dh =
            base64::engine::general_purpose::STANDARD.encode(ece::public_key_bytes(&secret));
        let auth = base64::engine::general_purpose::STANDARD.encode([250u8; 16]);
        let device = validate(
            &policy(),
            web_push("https://web.push.apple.com/abc", &p256dh, &auth),
        )
        .unwrap();
        assert_eq!(device.p256dh.unwrap(), ece::public_key_bytes(&secret));
    }

    #[test]
    fn native_tokens_are_validated_and_normalized() {
        let fcm = |token: &str| RegisterDeviceRequest {
            token: Some(Token::FcmToken(token.into())),
            ..Default::default()
        };
        let device = validate(&policy(), fcm(" cXyz:APA91b-token_123 ")).unwrap();
        assert_eq!(
            (device.channel, device.platform, device.token.as_str()),
            (
                PushChannelKind::PUSH_CHANNEL_KIND_FCM,
                Platform::PLATFORM_ANDROID,
                "cXyz:APA91b-token_123"
            )
        );
        assert_eq!(reason(fcm("")), "TOKEN_INVALID");
        assert_eq!(reason(fcm("has space")), "TOKEN_INVALID");
        assert_eq!(reason(fcm(&"a".repeat(MAX_TOKEN_LEN + 1))), "TOKEN_INVALID");

        let apns = |token: &str, sandbox: bool| RegisterDeviceRequest {
            token: Some(Token::from(ApnsToken {
                token: token.into(),
                sandbox,
                ..Default::default()
            })),
            ..Default::default()
        };
        let device = validate(&policy(), apns(&"AB".repeat(32), true)).unwrap();
        assert_eq!(
            (device.platform, device.token, device.apns_sandbox),
            (Platform::PLATFORM_IOS, "ab".repeat(32), true)
        );
        assert_eq!(reason(apns("abcd", false)), "TOKEN_INVALID");
        assert_eq!(reason(apns(&"zz".repeat(32), false)), "TOKEN_INVALID");
    }

    #[test]
    fn common_fields_are_validated() {
        let (p256dh, auth) = valid_keys();
        let base = || web_push("https://fcm.googleapis.com/fcm/send/abc", &p256dh, &auth);
        assert_eq!(reason(RegisterDeviceRequest::default()), "TOKEN_REQUIRED");
        assert_eq!(
            reason(RegisterDeviceRequest {
                locale: "not a locale!".into(),
                ..base()
            }),
            "LOCALE_INVALID"
        );
        assert_eq!(
            reason(RegisterDeviceRequest {
                device_name: "x".repeat(101),
                ..base()
            }),
            "DEVICE_NAME_TOO_LONG"
        );
        let device = validate(
            &policy(),
            RegisterDeviceRequest {
                locale: "en-US".into(),
                device_name: " Pixel 10 ".into(),
                platform: Platform::PLATFORM_DESKTOP.into(),
                ..base()
            },
        )
        .unwrap();
        assert_eq!(
            (
                device.locale.as_str(),
                device.device_name.as_str(),
                device.platform
            ),
            ("en-US", "Pixel 10", Platform::PLATFORM_DESKTOP)
        );
    }
}
