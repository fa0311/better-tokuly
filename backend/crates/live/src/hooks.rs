//! The HTTP hook the media server calls: `POST /hooks/live/auth`.
//!
//! MediaMTX (`authMethod: http`) asks before it lets anyone publish, read,
//! use the playback server or its control API. It sends
//! `{ user, password, token, ip, action, path, protocol, id, query, userAgent }`
//! and takes any 2xx answer as "yes". The decision:
//!
//! | `action` | Allowed when |
//! |---|---|
//! | `publish` | the path is a stream that is not over or suspended, and the credential is its stream key |
//! | `read` | the stream is on air and open to everyone with the link, or the request carries a valid playback token |
//! | `playback` | as `read`; the stream is on air with DVR on, and the requested start lies after the stream went on air and inside the DVR window |
//! | `api`, `metrics`, `pprof` | the caller presents the control API secret (the worker does) |
//!
//! Credentials arrive differently per protocol: RTMP puts `?token=` into
//! `token`, SRT its stream ID password into `password` (and `token`), WHIP
//! its bearer token into `token`; playback tokens come as the `t` query
//! parameter (HLS, WHEP and the playback server pass the query along) or as
//! a bearer token.
//!
//! Everything the encoder sends is recorded, also what it sends before the
//! stream goes on air (the owner's preview). Rewinding therefore never
//! reaches before `started_at`: the preview is not part of the broadcast.
//!
//! A read that was let in by a playback token is remembered with the viewer
//! the token was made for (`live:sessions:<stream>` in Valkey), so that the
//! session can be cut when that viewer loses access
//! ([`Live::enforce_access`]).
//!
//! # Who may call the hook
//!
//! Only the media server. It proves it with the hook secret
//! (`TOKULY_LIVE_HOOK_SECRET`) in the `Authorization` header: MediaMTX
//! sends `Basic` credentials when they are part of `authHTTPAddress`
//! (`http://mediamtx:<secret>@api:8081/hooks/live/auth`); `Bearer <secret>`
//! is accepted too. The secret is never read from the URL, which ends up in
//! access logs. An address that presents a wrong secret ten times in a
//! minute is refused outright for the rest of it. The hook secret is not
//! the control API's (`TOKULY_MEDIAMTX_API_SECRET`): whoever learns one
//! does not have the other. The route must not be published through the
//! edge.
use std::net::SocketAddr;

use axum::{
    Extension, Json, Router,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    routing::post,
};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use tokuly_core::{keys::secure_eq, prelude::*};

use crate::{
    Live,
    access::open_to_everyone,
    media_server::API_USER,
    streams::{self, StreamRow},
};

/// A start this much before the DVR window is still served (clock skew,
/// a player that asks a moment late).
const DVR_SLACK_SECONDS: i64 = 30;
/// Wrong secrets one address may present per minute before it is refused outright.
const MAX_FAILURES_PER_MINUTE: i64 = 10;

/// What the media server asks.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AuthRequest {
    pub user: String,
    pub password: String,
    pub token: String,
    pub ip: String,
    /// `publish`, `read`, `playback`, `api`, `metrics` or `pprof`.
    pub action: String,
    /// `live/<short_id>` for stream actions.
    pub path: String,
    pub protocol: String,
    /// The media server's ID of the connection or session that asks.
    pub id: Option<String>,
    /// The request's query string.
    pub query: String,
}

/// Why a request is refused (logged; the media server only sees the status).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    UnknownPath,
    UnknownStream,
    StreamOver,
    WrongKey,
    NotOnAir,
    NoToken,
    DvrOff,
    OutsideDvrWindow,
    NotTheWorker,
    UnknownAction,
}

fn sessions_key(stream_id: Uuid) -> String {
    format!("live:sessions:{stream_id}")
}

/// The secret in an `Authorization` header: the password of `Basic`
/// credentials, or a `Bearer` token.
fn presented_secret(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    if let Some(token) = value.strip_prefix("Bearer ") {
        return Some(token.trim().to_owned());
    }
    let encoded = value.strip_prefix("Basic ")?.trim();
    // Standard base64, as HTTP uses it, read with the URL-safe decoder.
    let url_safe: String = encoded
        .trim_end_matches('=')
        .chars()
        .map(|c| match c {
            '+' => '-',
            '/' => '_',
            other => other,
        })
        .collect();
    let decoded = String::from_utf8(tokuly_core::keys::b64url_decode(&url_safe)?).ok()?;
    decoded.split_once(':').map(|(_, password)| password.to_owned())
}

impl Live {
    /// Routes for the media server, to be mounted at `/hooks/live` on a
    /// listener the media server can reach. Not for the public edge.
    pub fn http_routes(&self) -> Router {
        Router::new().route("/auth", post(auth)).with_state(self.clone())
    }

    /// Whether a request to the hook presents the hook secret.
    fn hook_secret_matches(&self, presented: &str) -> bool {
        let secret = &self.cfg.hook_secret;
        !secret.is_empty() && secure_eq(secret.as_bytes(), presented.as_bytes())
    }

    /// Decides one question of the media server.
    pub async fn authorize_media(&self, request: &AuthRequest) -> AppResult<Result<(), Refusal>> {
        match request.action.as_str() {
            "publish" => self.authorize_publish(request).await,
            "read" => self.authorize_read(request, false).await,
            "playback" => self.authorize_read(request, true).await,
            "api" | "metrics" | "pprof" => {
                let secret = &self.cfg.mediamtx_api_secret;
                let ours = request.user == API_USER
                    && !secret.is_empty()
                    && secure_eq(secret.as_bytes(), request.password.as_bytes());
                Ok(if ours { Ok(()) } else { Err(Refusal::NotTheWorker) })
            }
            _ => Ok(Err(Refusal::UnknownAction)),
        }
    }

    async fn stream_of_path(&self, path: &str) -> AppResult<Result<StreamRow, Refusal>> {
        let Some(short_id) = streams::short_id_of_path(path) else { return Ok(Err(Refusal::UnknownPath)) };
        let stream = streams::find(&self.core, &tokuly_core::ids::Ref::Short(short_id.to_owned())).await?;
        Ok(stream.filter(|stream| !stream.suspended).ok_or(Refusal::UnknownStream))
    }

    async fn authorize_publish(&self, request: &AuthRequest) -> AppResult<Result<(), Refusal>> {
        let stream = match self.stream_of_path(&request.path).await? {
            Ok(stream) => stream,
            Err(refusal) => return Ok(Err(refusal)),
        };
        if stream.is_ended() {
            return Ok(Err(Refusal::StreamOver));
        }
        let key = if request.token.is_empty() { &request.password } else { &request.token };
        Ok(if streams::key_matches(&self.core, &stream, key) { Ok(()) } else { Err(Refusal::WrongKey) })
    }

    async fn authorize_read(&self, request: &AuthRequest, dvr: bool) -> AppResult<Result<(), Refusal>> {
        let stream = match self.stream_of_path(&request.path).await? {
            Ok(stream) => stream,
            Err(refusal) => return Ok(Err(refusal)),
        };
        if stream.is_ended() {
            return Ok(Err(Refusal::StreamOver));
        }
        let query = parse_query(&request.query);
        let param = |name: &str| query.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str());
        let presented = [param("t"), Some(request.token.as_str()), Some(request.password.as_str())];
        let viewer =
            presented.into_iter().flatten().find_map(|token| self.playback_token_viewer(&stream.short_id, token));
        if viewer.is_none() {
            // Without a token only what everyone may watch, and only on air.
            if !open_to_everyone(stream.visibility) {
                return Ok(Err(Refusal::NoToken));
            }
            if !stream.is_live() {
                return Ok(Err(Refusal::NotOnAir));
            }
        }
        if dvr {
            if !stream.dvr_enabled {
                return Ok(Err(Refusal::DvrOff));
            }
            // Only the broadcast can be rewound: not the preview before it
            // (which is recorded too), and never further than the window.
            let Some(started_at) = stream.started_at.filter(|_| stream.is_live()) else {
                return Ok(Err(Refusal::NotOnAir));
            };
            if let Some(start) = param("start") {
                let earliest =
                    started_at.max(now() - Duration::seconds(self.cfg.dvr_window_seconds + DVR_SLACK_SECONDS));
                match DateTime::parse_from_rfc3339(start) {
                    Ok(start) if start.with_timezone(&Utc) >= earliest => {}
                    _ => return Ok(Err(Refusal::OutsideDvrWindow)),
                }
            }
        }
        // Remember whose session this is, so it can be cut when they lose access.
        if let (Some(viewer), Some(session), false) = (viewer, request.id.as_deref().filter(|id| !id.is_empty()), dvr) {
            let key = sessions_key(stream.id);
            redis::pipe()
                .cmd("HSET")
                .arg(&key)
                .arg(session)
                .arg(viewer.to_string())
                .ignore()
                .cmd("EXPIRE")
                .arg(&key)
                .arg(86_400)
                .ignore()
                .query_async::<()>(&mut self.core.cache.conn())
                .await?;
        }
        Ok(Ok(()))
    }

    /// The viewers behind the media sessions of a stream that were let in
    /// by a playback token, by session ID.
    pub(crate) async fn token_sessions(&self, stream_id: Uuid) -> AppResult<std::collections::HashMap<String, Uuid>> {
        let entries: Vec<(String, String)> =
            redis::cmd("HGETALL").arg(sessions_key(stream_id)).query_async(&mut self.core.cache.conn()).await?;
        Ok(entries.into_iter().filter_map(|(session, viewer)| Some((session, viewer.parse().ok()?))).collect())
    }

    pub(crate) async fn forget_sessions(&self, stream_id: Uuid, sessions: &[String]) -> AppResult<()> {
        if sessions.is_empty() {
            return Ok(());
        }
        redis::cmd("HDEL")
            .arg(sessions_key(stream_id))
            .arg(sessions)
            .query_async::<()>(&mut self.core.cache.conn())
            .await?;
        Ok(())
    }

    /// Whether `peer` has presented too many wrong secrets lately.
    async fn locked_out(&self, peer: &str) -> bool {
        let count: Option<i64> = redis::cmd("GET")
            .arg(format!("live:hook-failures:{peer}"))
            .query_async(&mut self.core.cache.conn())
            .await
            .unwrap_or(None);
        count.unwrap_or(0) >= MAX_FAILURES_PER_MINUTE
    }

    async fn note_failure(&self, peer: &str) {
        let key = format!("live:hook-failures:{peer}");
        let mut conn = self.core.cache.conn();
        let counted: Result<i64, _> = redis::cmd("INCR").arg(&key).query_async(&mut conn).await;
        if counted.is_ok_and(|count| count == 1) {
            let _: Result<(), _> = redis::cmd("EXPIRE").arg(&key).arg(60).query_async(&mut conn).await;
        }
    }
}

/// Splits a query string into decoded pairs.
pub fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .trim_start_matches('?')
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(key), percent_decode(value))
        })
        .collect()
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = (bytes[i] == b'%')
            .then(|| bytes.get(i + 1..i + 3))
            .flatten()
            .filter(|hex| hex.iter().all(u8::is_ascii_hexdigit))
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match (escaped, bytes[i]) {
            (Some(byte), _) => {
                out.push(byte);
                i += 3;
            }
            (None, b'+') => {
                out.push(b' ');
                i += 1;
            }
            (None, other) => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

async fn auth(
    State(live): State<Live>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Json(request): Json<AuthRequest>,
) -> StatusCode {
    let peer =
        peer.map(|Extension(ConnectInfo(address))| address.ip().to_string()).unwrap_or_else(|| "unknown".to_owned());
    if live.locked_out(&peer).await {
        return StatusCode::TOO_MANY_REQUESTS;
    }
    let authentic = presented_secret(&headers).is_some_and(|secret| live.hook_secret_matches(&secret));
    if !authentic {
        live.note_failure(&peer).await;
        return StatusCode::FORBIDDEN;
    }
    match live.authorize_media(&request).await {
        Ok(Ok(())) => StatusCode::OK,
        Ok(Err(refusal)) => {
            tracing::debug!(action = %request.action, path = %request.path, protocol = %request.protocol, ?refusal, "media request refused");
            StatusCode::UNAUTHORIZED
        }
        Err(error) => {
            tracing::error!(action = %request.action, path = %request.path, %error, "media authorization failed");
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_strings_are_split_and_decoded() {
        let query = parse_query("path=live%2FabcDEF12345&start=2026-11-22T09%3A33%3A39%2B09%3A00&t=17.x_y-z&flag");
        assert_eq!(
            query,
            vec![
                ("path".to_owned(), "live/abcDEF12345".to_owned()),
                ("start".to_owned(), "2026-11-22T09:33:39+09:00".to_owned()),
                ("t".to_owned(), "17.x_y-z".to_owned()),
                ("flag".to_owned(), String::new()),
            ]
        );
        assert!(parse_query("").is_empty());
        // Broken escapes are kept as written.
        assert_eq!(parse_query("a=100%&b=%zz&c=%4")[0].1, "100%");
        assert_eq!(parse_query("a=100%&b=%zz&c=%4")[1].1, "%zz");
        assert_eq!(parse_query("a=100%&b=%zz&c=%4")[2].1, "%4");
        assert_eq!(parse_query("?q=a+b")[0].1, "a b");
    }

    #[test]
    fn the_secret_is_read_from_the_authorization_header() {
        let with = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(AUTHORIZATION, value.parse().unwrap());
            presented_secret(&headers)
        };
        // "mediamtx:s3cret-hook", as MediaMTX sends credentials from its hook URL.
        assert_eq!(with("Basic bWVkaWFtdHg6czNjcmV0LWhvb2s=").as_deref(), Some("s3cret-hook"));
        assert_eq!(with("Bearer abc").as_deref(), Some("abc"));
        assert_eq!(with("Basic !!!"), None);
        assert_eq!(with("Digest x"), None);
        assert_eq!(presented_secret(&HeaderMap::new()), None);
    }
}
