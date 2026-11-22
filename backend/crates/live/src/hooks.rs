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
//! | `playback` | as `read`, DVR is on, and the requested start lies inside the DVR window |
//! | `api`, `metrics`, `pprof` | the caller presents the hook secret (the worker does) |
//!
//! Credentials arrive differently per protocol: RTMP puts `?token=` into
//! `token`, SRT its stream ID password into `password` (and `token`), WHIP
//! its bearer token into `token`; playback tokens come as the `t` query
//! parameter (HLS, WHEP and the playback server pass the query along) or as
//! a bearer token.
//!
//! The route itself is protected by the shared secret
//! (`TOKULY_LIVE_HOOK_SECRET`): in the `secret` query parameter, as the
//! media server's `authHTTPAddress` carries it, or as a bearer token. It
//! must not be published through the edge; it is for the media server only.
use axum::{
    Json, Router,
    extract::{Query, State},
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

#[derive(Deserialize)]
struct HookQuery {
    #[serde(default)]
    secret: String,
}

impl Live {
    /// Routes for the media server, to be mounted at `/hooks/live` on a
    /// listener the media server can reach. Not for the public edge.
    pub fn http_routes(&self) -> Router {
        Router::new().route("/auth", post(auth)).with_state(self.clone())
    }

    /// Whether a request to the hook presents the shared secret.
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
                let ours = request.user == API_USER && self.hook_secret_matches(&request.password);
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
        let has_token = presented
            .into_iter()
            .flatten()
            .any(|token| !token.is_empty() && self.playback_token_valid(&stream.short_id, token));
        if !has_token {
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
            if let Some(start) = param("start") {
                let earliest = now() - Duration::seconds(self.cfg.dvr_window_seconds + DVR_SLACK_SECONDS);
                match DateTime::parse_from_rfc3339(start) {
                    Ok(start) if start.with_timezone(&Utc) >= earliest => {}
                    _ => return Ok(Err(Refusal::OutsideDvrWindow)),
                }
            }
        }
        Ok(Ok(()))
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
    Query(hook): Query<HookQuery>,
    headers: HeaderMap,
    Json(request): Json<AuthRequest>,
) -> StatusCode {
    let bearer = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();
    if !live.hook_secret_matches(&hook.secret) && !live.hook_secret_matches(bearer) {
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
}
