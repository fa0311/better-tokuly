//! Sessions and refresh-token rotation.
//!
//! A session is one signed-in device. Its refresh token is opaque
//! (`<session id>.<random>`), stored only as a keyed hash, and rotated on
//! every use. Presenting a token that is neither current nor the one just
//! replaced means it was stolen and replayed, so the whole session is revoked.
use chrono::{DateTime, Duration, Utc};
use tokuly_core::{auth::ClientMeta, keys::random_token, prelude::*};
use tokuly_proto::identity::{ClientInfo, Session};

use crate::{Identity, users::UserRow};

/// Name of the HttpOnly cookie holding the refresh token for web clients.
pub const REFRESH_COOKIE: &str = "tokuly_rt";
/// RPC path the cookie is scoped to (below the public API path prefix):
/// only the auth service ever receives the refresh token.
const COOKIE_RPC_PATH: &str = "/tokuly.identity.v1.AuthService/";
/// How long the previous refresh token is still honoured after a rotation.
const ROTATION_GRACE_SECONDS: i64 = 60;

/// Freshly issued credentials for a session.
#[derive(Debug, Clone)]
pub struct Issued {
    pub session_id: Uuid,
    pub access_token: String,
    pub access_expires_at: DateTime<Utc>,
    pub refresh_token: String,
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: Uuid,
    user_id: Uuid,
    refresh_hash: String,
    refresh_sealed: Vec<u8>,
    rotated_at: Option<DateTime<Utc>>,
    expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
}

fn new_refresh_token(session_id: Uuid) -> String {
    format!("{}.{}", session_id.simple(), random_token(32))
}

/// The session ID embedded in a refresh token, if it is well formed.
fn session_id_of(token: &str) -> Option<Uuid> {
    token
        .split_once('.')
        .and_then(|(id, _)| Uuid::parse_str(id).ok())
}

impl Identity {
    /// Creates a session for `user` and issues its first token pair.
    pub async fn create_session(
        &self,
        user: &UserRow,
        client: Option<&ClientInfo>,
        meta: &ClientMeta,
    ) -> AppResult<Issued> {
        let core = &self.core;
        let session_id = new_id();
        let refresh_token = new_refresh_token(session_id);
        let default_client = ClientInfo::default();
        let client = client.unwrap_or(&default_client);
        let device_name = if client.device_name.is_empty() {
            device_name_from_user_agent(&meta.user_agent)
        } else {
            truncate(&client.device_name, 80)
        };

        let mut tx = core.db.begin().await?;
        if !client.device_id.is_empty() {
            // One session per install: signing in again replaces the old one.
            let replaced: Vec<Uuid> = sqlx::query_scalar(
                "UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND device_id = $2 AND revoked_at IS NULL RETURNING id",
            )
            .bind(user.id)
            .bind(&client.device_id)
            .fetch_all(&mut *tx)
            .await?;
            for id in replaced {
                core.revoke_session(id).await;
            }
        }
        sqlx::query(
            "INSERT INTO sessions (id, user_id, refresh_hash, refresh_sealed, platform, device_name, device_id, app_version, ip, user_agent, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(session_id)
        .bind(user.id)
        .bind(core.keys.token_hash(&refresh_token))
        .bind(core.keys.seal("refresh", refresh_token.as_bytes()))
        .bind(client.platform.to_i32())
        .bind(&device_name)
        .bind(truncate(&client.device_id, 128))
        .bind(truncate(&client.app_version, 32))
        .bind(&meta.ip)
        .bind(truncate(&meta.user_agent, 400))
        .bind(Utc::now() + Duration::days(self.cfg.session_days))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        let (access_token, access_expires_at) =
            core.tokens.issue(user.id, session_id, &user.roles)?;
        Ok(Issued {
            session_id,
            access_token,
            access_expires_at,
            refresh_token,
        })
    }

    /// Exchanges a refresh token for a new token pair.
    pub async fn refresh_session(
        &self,
        presented: &str,
        meta: &ClientMeta,
    ) -> AppResult<(Issued, UserRow)> {
        let core = &self.core;
        let invalid = || AppError::unauthenticated("REFRESH_INVALID", "sign in again");
        if presented.is_empty() {
            return Err(invalid());
        }
        let hash = core.keys.token_hash(presented);
        let row: Option<SessionRow> = sqlx::query_as(
            "SELECT id, user_id, refresh_hash, refresh_sealed, rotated_at, expires_at, revoked_at
             FROM sessions WHERE refresh_hash = $1 OR previous_refresh_hash = $1",
        )
        .bind(&hash)
        .fetch_optional(&core.db)
        .await?;

        let Some(row) = row else {
            // Unknown token for a live session: an old token is being replayed.
            if let Some(session_id) = session_id_of(presented) {
                let owner: Option<Uuid> = sqlx::query_scalar(
                    "UPDATE sessions SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL RETURNING user_id",
                )
                .bind(session_id)
                .fetch_optional(&core.db)
                .await?;
                if let Some(user_id) = owner {
                    core.revoke_session(session_id).await;
                    self.log_security_event(
                        user_id,
                        "REFRESH_TOKEN_REUSE",
                        meta,
                        "",
                        serde_json::json!({ "session_id": session_id }),
                    )
                    .await;
                    tracing::warn!(%user_id, %session_id, "refresh token reuse detected; session revoked");
                }
            }
            return Err(invalid());
        };
        if row.revoked_at.is_some() || row.expires_at <= Utc::now() {
            return Err(AppError::unauthenticated(
                "SESSION_EXPIRED",
                "sign in again",
            ));
        }
        let user = crate::users::get(core, row.user_id)
            .await
            .map_err(|_| invalid())?;
        if user.deleted_at.is_some() {
            return Err(invalid());
        }
        if user.banned_at.is_some() {
            return Err(AppError::denied(
                "ACCOUNT_BANNED",
                "this account has been suspended",
            ));
        }

        let refresh_token = if row.refresh_hash == hash {
            // Normal path: rotate.
            let next = new_refresh_token(row.id);
            let updated = sqlx::query(
                "UPDATE sessions
                 SET previous_refresh_hash = refresh_hash, refresh_hash = $2, refresh_sealed = $3, rotated_at = now(),
                     last_active_at = now(), expires_at = $4, ip = $5
                 WHERE id = $1 AND refresh_hash = $6 AND revoked_at IS NULL",
            )
            .bind(row.id)
            .bind(core.keys.token_hash(&next))
            .bind(core.keys.seal("refresh", next.as_bytes()))
            .bind(Utc::now() + Duration::days(self.cfg.session_days))
            .bind(&meta.ip)
            .bind(&hash)
            .execute(&core.db)
            .await?;
            if updated.rows_affected() == 1 {
                next
            } else {
                // Lost a race with a concurrent refresh; hand out what that one issued.
                self.current_refresh_token(row.id)
                    .await?
                    .ok_or_else(invalid)?
            }
        } else {
            // The previous token: fine shortly after a rotation, theft afterwards.
            let within_grace = row
                .rotated_at
                .is_some_and(|t| Utc::now() - t <= Duration::seconds(ROTATION_GRACE_SECONDS));
            if !within_grace {
                sqlx::query(
                    "UPDATE sessions SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL",
                )
                .bind(row.id)
                .execute(&core.db)
                .await?;
                core.revoke_session(row.id).await;
                self.log_security_event(
                    row.user_id,
                    "REFRESH_TOKEN_REUSE",
                    meta,
                    "",
                    serde_json::json!({ "session_id": row.id }),
                )
                .await;
                return Err(invalid());
            }
            String::from_utf8(
                core.keys
                    .open("refresh", &row.refresh_sealed)
                    .ok_or_else(invalid)?,
            )
            .map_err(|_| invalid())?
        };

        let (access_token, access_expires_at) = core.tokens.issue(user.id, row.id, &user.roles)?;
        Ok((
            Issued {
                session_id: row.id,
                access_token,
                access_expires_at,
                refresh_token,
            },
            user,
        ))
    }

    async fn current_refresh_token(&self, session_id: Uuid) -> AppResult<Option<String>> {
        let sealed: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT refresh_sealed FROM sessions WHERE id = $1 AND revoked_at IS NULL",
        )
        .bind(session_id)
        .fetch_optional(&self.core.db)
        .await?;
        Ok(sealed
            .and_then(|s| self.core.keys.open("refresh", &s))
            .and_then(|b| String::from_utf8(b).ok()))
    }

    /// Revokes one session of a user. Returns whether it existed.
    pub async fn revoke_session(&self, user_id: Uuid, session_id: Uuid) -> AppResult<bool> {
        let done = sqlx::query("UPDATE sessions SET revoked_at = now() WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL")
            .bind(session_id)
            .bind(user_id)
            .execute(&self.core.db)
            .await?;
        self.core.revoke_session(session_id).await;
        Ok(done.rows_affected() > 0)
    }

    /// Revokes every session of a user except `keep`.
    pub async fn revoke_all_sessions(&self, user_id: Uuid, keep: Option<Uuid>) -> AppResult<u64> {
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL AND ($2::uuid IS NULL OR id <> $2) RETURNING id",
        )
        .bind(user_id)
        .bind(keep)
        .fetch_all(&self.core.db)
        .await?;
        for id in &ids {
            self.core.revoke_session(*id).await;
        }
        Ok(ids.len() as u64)
    }

    pub async fn list_sessions(&self, user_id: Uuid, current: Uuid) -> AppResult<Vec<Session>> {
        #[derive(sqlx::FromRow)]
        struct Row {
            id: Uuid,
            platform: i32,
            device_name: String,
            device_id: String,
            app_version: String,
            ip: String,
            location: String,
            created_at: DateTime<Utc>,
            last_active_at: DateTime<Utc>,
        }
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT id, platform, device_name, device_id, app_version, ip, location, created_at, last_active_at
             FROM sessions WHERE user_id = $1 AND revoked_at IS NULL AND expires_at > now()
             ORDER BY last_active_at DESC",
        )
        .bind(user_id)
        .fetch_all(&self.core.db)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| Session {
                id: r.id.to_string(),
                client: ClientInfo {
                    platform: r.platform.into(),
                    device_name: r.device_name,
                    app_version: r.app_version,
                    device_id: r.device_id,
                    ..Default::default()
                }
                .into(),
                ip: r.ip,
                location: r.location,
                created_at: ts(r.created_at),
                last_active_at: ts(r.last_active_at),
                current: r.id == current,
                ..Default::default()
            })
            .collect())
    }

    /// Appends to the account's security log. Never fails the caller.
    pub async fn log_security_event(
        &self,
        user_id: Uuid,
        kind: &str,
        meta: &ClientMeta,
        device_name: &str,
        attrs: serde_json::Value,
    ) {
        let device = if device_name.is_empty() {
            device_name_from_user_agent(&meta.user_agent)
        } else {
            device_name.to_owned()
        };
        let result = sqlx::query("INSERT INTO security_events (user_id, type, ip, device_name, attrs) VALUES ($1, $2, $3, $4, $5)")
            .bind(user_id)
            .bind(kind)
            .bind(&meta.ip)
            .bind(device)
            .bind(attrs)
            .execute(&self.core.db)
            .await;
        if let Err(e) = result {
            tracing::warn!(%user_id, kind, error = %e, "failed to record security event");
        }
    }

    fn refresh_cookie_path(&self) -> String {
        format!("{}{COOKIE_RPC_PATH}", self.core.cfg.api_path_prefix())
    }

    /// `Set-Cookie` value storing the refresh token for web clients.
    pub fn refresh_cookie(&self, token: &str) -> String {
        let secure = if self.core.cfg.public_url.starts_with("https://") {
            "; Secure"
        } else {
            ""
        };
        let max_age = self.cfg.session_days * 24 * 3600;
        let path = self.refresh_cookie_path();
        format!("{REFRESH_COOKIE}={token}; Path={path}; Max-Age={max_age}; HttpOnly; SameSite=Lax{secure}")
    }

    /// `Set-Cookie` value removing the refresh cookie.
    pub fn clear_refresh_cookie(&self) -> String {
        let secure = if self.core.cfg.public_url.starts_with("https://") {
            "; Secure"
        } else {
            ""
        };
        let path = self.refresh_cookie_path();
        format!("{REFRESH_COOKIE}=; Path={path}; Max-Age=0; HttpOnly; SameSite=Lax{secure}")
    }
}

/// Finds a cookie in a raw `Cookie` header.
pub fn cookie_value<'a>(cookies: &'a str, name: &str) -> Option<&'a str> {
    cookies
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

/// A rough "Browser on OS" label for sessions that did not name their device.
pub fn device_name_from_user_agent(ua: &str) -> String {
    if ua.is_empty() {
        return "Unknown device".to_owned();
    }
    let browser = [
        ("Edg/", "Edge"),
        ("OPR/", "Opera"),
        ("Firefox/", "Firefox"),
        ("Chrome/", "Chrome"),
        ("Safari/", "Safari"),
    ]
    .iter()
    .find(|(needle, _)| ua.contains(needle))
    .map(|(_, name)| *name);
    let os = [
        ("iPhone", "iPhone"),
        ("iPad", "iPad"),
        ("Android", "Android"),
        ("Windows", "Windows"),
        ("Mac OS X", "macOS"),
        ("CrOS", "ChromeOS"),
        ("Linux", "Linux"),
    ]
    .iter()
    .find(|(needle, _)| ua.contains(needle))
    .map(|(_, name)| *name);
    match (browser, os) {
        (Some(b), Some(o)) => format!("{b} on {o}"),
        (Some(b), None) => b.to_owned(),
        (None, Some(o)) => o.to_owned(),
        (None, None) => truncate(ua, 40),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_lookup() {
        let header = "theme=dark; tokuly_rt=abc.def; other=1";
        assert_eq!(cookie_value(header, REFRESH_COOKIE), Some("abc.def"));
        assert_eq!(cookie_value(header, "missing"), None);
        assert_eq!(cookie_value("", REFRESH_COOKIE), None);
    }

    #[test]
    fn refresh_tokens_embed_their_session() {
        let id = Uuid::now_v7();
        let token = new_refresh_token(id);
        assert_eq!(session_id_of(&token), Some(id));
        assert_eq!(session_id_of("garbage"), None);
        assert_ne!(token, new_refresh_token(id));
    }

    #[test]
    fn user_agents_become_readable_device_names() {
        let chrome_mac = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/157.0.0.0 Safari/537.36";
        assert_eq!(device_name_from_user_agent(chrome_mac), "Chrome on macOS");
        let safari_ios = "Mozilla/5.0 (iPhone; CPU iPhone OS 27_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/27.0 Mobile/15E148 Safari/604.1";
        assert_eq!(device_name_from_user_agent(safari_ios), "Safari on iPhone");
        assert_eq!(device_name_from_user_agent(""), "Unknown device");
    }
}
