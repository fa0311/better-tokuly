//! The media server (MediaMTX): what clients are told about it, and its
//! control API.
//!
//! Every stream is the path `live/<short_id>`:
//!
//! | What | URL |
//! |---|---|
//! | RTMP ingest | `<rtmp>/live/<short_id>?token=<stream key>` |
//! | SRT ingest | `<srt>?streamid=publish:live/<short_id>:tokuly:<stream key>` |
//! | WHIP ingest | `<whip>/live/<short_id>/whip`, `Authorization: Bearer <stream key>` |
//! | LL-HLS playback | `<hls>/live/<short_id>/index.m3u8[?t=<playback token>]` |
//! | WHEP playback | `<whep>/live/<short_id>/whep[?t=<playback token>]` |
//! | DVR | `<playback>/list?path=live/<short_id>` and `/get?path=…&start=…&duration=…` |
//!
//! The media server asks [`crate::hooks`] about every one of these. Streams
//! that anyone with the link may watch need no token; friends-only and
//! private streams, and the owner's preview before going on air, do. A
//! playback token is a keyed hash of the stream's name, the viewer it was
//! made for and an expiry a quarter of an hour ahead, so checking it needs
//! no storage; players get a fresh one with every heartbeat
//! (`ReportWatching`). Because a token names its viewer, the session it
//! opened can be cut when that viewer loses access ([`Live::enforce_access`]).
//!
//! The control API is called with its own secret as basic-auth password
//! (`TOKULY_MEDIAMTX_API_SECRET`, not the hook's), which the media server
//! in turn has checked by the hook (action `api`).
use chrono::Duration;
use serde::Deserialize;
use tokuly_core::{
    keys::{b64url, secure_eq},
    prelude::*,
};
use tokuly_proto::live::{IngestInfo, Playback};

use crate::{
    Live,
    streams::{self, StreamRow},
};

/// User name the worker presents to the control API.
pub const API_USER: &str = "tokuly";
/// How long playback URLs of restricted streams stay valid.
pub const PLAYBACK_TOKEN_TTL_MINUTES: i64 = 15;
/// Paths asked for per page of the control API.
const PAGE_SIZE: usize = 500;

/// A path as the media server reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaPath {
    /// `live/<short_id>`.
    pub name: String,
    /// A publisher is connected and sending.
    pub online: bool,
    /// Kind of the publisher's connection (`rtmpConn`, `srtConn`,
    /// `webRTCSession`) and its ID, for kicking it.
    pub source_type: String,
    pub source_id: String,
    /// Connections reading the path (each HLS viewer is one).
    pub readers: i64,
    /// Those connections as `(type, ID)`, for cutting them.
    pub reader_ids: Vec<(String, String)>,
}

impl MediaPath {
    /// `rtmp`, `srt` or `webrtc`.
    pub fn protocol(&self) -> &'static str {
        protocol_of(&self.source_type)
    }
}

/// The protocol behind a media server connection type.
pub fn protocol_of(source_type: &str) -> &'static str {
    match source_type {
        "rtmpConn" | "rtmpsConn" => "rtmp",
        "srtConn" => "srt",
        "webRTCSession" => "webrtc",
        _ => "",
    }
}

/// The control API endpoint that disconnects a connection of this type.
fn kick_endpoint(source_type: &str) -> Option<&'static str> {
    match source_type {
        "rtmpConn" => Some("rtmp/conns"),
        "rtmpsConn" => Some("rtmps/conns"),
        "srtConn" => Some("srt/conns"),
        "webRTCSession" => Some("webrtc/sessions"),
        "hlsSession" => Some("hls/sessions"),
        _ => None,
    }
}

#[derive(Deserialize)]
struct PathList {
    #[serde(default, rename = "pageCount")]
    page_count: usize,
    #[serde(default)]
    items: Vec<PathItem>,
}

#[derive(Deserialize)]
struct PathItem {
    name: String,
    #[serde(default)]
    online: bool,
    #[serde(default)]
    source: Option<PathSource>,
    #[serde(default)]
    readers: Vec<PathSource>,
}

#[derive(Deserialize)]
struct PathSource {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    id: String,
}

impl From<PathItem> for MediaPath {
    fn from(item: PathItem) -> Self {
        let (source_type, source_id) = item.source.map(|s| (s.kind, s.id)).unwrap_or_default();
        let reader_ids: Vec<(String, String)> =
            item.readers.into_iter().map(|reader| (reader.kind, reader.id)).collect();
        Self {
            name: item.name,
            online: item.online,
            source_type,
            source_id,
            readers: reader_ids.len() as i64,
            reader_ids,
        }
    }
}

fn unreachable(error: impl std::fmt::Display) -> AppError {
    AppError::unavailable("MEDIA_SERVER_UNREACHABLE", format!("the media server did not answer: {error}"))
}

impl Live {
    /// Refuses with `MEDIA_SERVER_NOT_CONFIGURED` when no media server is
    /// configured: nothing can go on air then.
    pub(crate) fn require_media_server(&self) -> AppResult<()> {
        ensure!(
            self.cfg.media_server_configured(),
            unavailable,
            "MEDIA_SERVER_NOT_CONFIGURED",
            "live streaming is not set up on this server"
        );
        Ok(())
    }

    fn api_url(&self, path: &str) -> String {
        format!("{}/v3/{}", self.cfg.mediamtx_api_url.trim_end_matches('/'), path)
    }

    /// Every path the media server knows, with who publishes and how many read.
    pub async fn media_paths(&self) -> AppResult<Vec<MediaPath>> {
        self.require_media_server()?;
        let mut paths = Vec::new();
        let mut page = 0;
        loop {
            let response = self
                .core
                .http
                .get(self.api_url("paths/list"))
                .query(&[("page", page.to_string()), ("itemsPerPage", PAGE_SIZE.to_string())])
                .basic_auth(API_USER, Some(&self.cfg.mediamtx_api_secret))
                .send()
                .await
                .map_err(unreachable)?;
            if !response.status().is_success() {
                return Err(unreachable(format!("paths/list answered {}", response.status())));
            }
            let list: PathList = response.json().await.map_err(unreachable)?;
            paths.extend(list.items.into_iter().map(MediaPath::from));
            page += 1;
            if page >= list.page_count {
                return Ok(paths);
            }
        }
    }

    /// One path, or `None` when the media server does not have it.
    pub async fn media_path(&self, name: &str) -> AppResult<Option<MediaPath>> {
        self.require_media_server()?;
        let response = self
            .core
            .http
            .get(self.api_url(&format!("paths/get/{name}")))
            .basic_auth(API_USER, Some(&self.cfg.mediamtx_api_secret))
            .send()
            .await
            .map_err(unreachable)?;
        if response.status().as_u16() == 404 {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(unreachable(format!("paths/get answered {}", response.status())));
        }
        let item: PathItem = response.json().await.map_err(unreachable)?;
        Ok(Some(item.into()))
    }

    /// Disconnects whoever publishes to a path. Returns whether there was
    /// someone to disconnect.
    pub async fn kick_publisher(&self, name: &str) -> AppResult<bool> {
        let Some(path) = self.media_path(name).await? else { return Ok(false) };
        self.kick(&path).await
    }

    pub(crate) async fn kick(&self, path: &MediaPath) -> AppResult<bool> {
        self.kick_connection(&path.source_type, &path.source_id).await
    }

    /// Disconnects one connection or session (a publisher or a reader).
    pub(crate) async fn kick_connection(&self, kind: &str, id: &str) -> AppResult<bool> {
        let Some(endpoint) = kick_endpoint(kind) else { return Ok(false) };
        let response = self
            .core
            .http
            .post(self.api_url(&format!("{endpoint}/kick/{id}")))
            .basic_auth(API_USER, Some(&self.cfg.mediamtx_api_secret))
            .send()
            .await
            .map_err(unreachable)?;
        // 404: the connection went away by itself in the meantime.
        Ok(response.status().is_success())
    }

    /// Disconnects a stream's encoder, without failing the caller: the
    /// reconciler kicks whatever is left on the next tick anyway.
    pub(crate) async fn kick_quietly(&self, stream: &StreamRow) {
        if !self.cfg.media_server_configured() {
            return;
        }
        if let Err(error) = self.kick_publisher(&stream.media_path()).await {
            tracing::warn!(stream_id = %stream.id, %error, "could not disconnect the encoder");
        }
    }

    // ───────────────────────────── playback ─────────────────────────────

    /// A token that opens the playback URLs of `short_id` for `viewer`
    /// until `expires`. It names the viewer it was made for, so the media
    /// session it opens can be cut when that viewer loses access.
    pub fn playback_token(&self, short_id: &str, viewer: Uuid, expires: Time) -> String {
        let expiry = expires.timestamp();
        format!("{expiry}.{}.{}", viewer.simple(), self.playback_signature(short_id, viewer, expiry))
    }

    fn playback_signature(&self, short_id: &str, viewer: Uuid, expiry: i64) -> String {
        let mac = self.core.keys.mac("live/playback", format!("{short_id}.{}.{expiry}", viewer.simple()).as_bytes());
        b64url(&mac[..16])
    }

    /// The viewer `token` was made for, if it opens `short_id` right now.
    pub fn playback_token_viewer(&self, short_id: &str, token: &str) -> Option<Uuid> {
        let mut parts = token.splitn(3, '.');
        let expiry: i64 = parts.next()?.parse().ok()?;
        let viewer = Uuid::parse_str(parts.next()?).ok()?;
        let signature = parts.next()?;
        let valid = expiry > now().timestamp()
            && secure_eq(self.playback_signature(short_id, viewer, expiry).as_bytes(), signature.as_bytes());
        valid.then_some(viewer)
    }

    /// Cuts the media sessions of those who may not (or no longer) watch a
    /// restricted stream. A token is only checked when a session is opened
    /// (the hook does that, on every request it gets), so a session that
    /// is already open is cut here, and whoever still may watch reconnects
    /// through the hook.
    ///
    /// Every reader whose session the hook remembers is judged by its
    /// viewer: a viewer who lost access, or nobody (the session was opened
    /// while the stream was open to everyone), goes. A reader the hook has
    /// no record of is cut only when `cut_unknown` is set: when something
    /// about who may watch has just changed (visibility, a friendship, a
    /// block) it cannot be shown to be allowed, so it goes and its player
    /// comes back through the hook; the regular pass leaves it alone, so
    /// that lost bookkeeping never throws legitimate viewers out again and
    /// again. Returns how many were cut.
    pub async fn enforce_access(&self, stream: &StreamRow, cut_unknown: bool) -> AppResult<u32> {
        if !self.cfg.media_server_configured() || (stream.is_live() && stream.open_to_everyone()) {
            return Ok(0);
        }
        let Some(path) = self.media_path(&stream.media_path()).await? else { return Ok(0) };
        let sessions = self.known_sessions(stream.id).await?;
        let mut allowed: std::collections::HashMap<Uuid, bool> = std::collections::HashMap::new();
        let mut cut = 0;
        let mut gone = Vec::new();
        for (kind, id) in &path.reader_ids {
            let may_stay = match sessions.get(id) {
                None => !cut_unknown,
                Some(None) => false,
                Some(Some(viewer_id)) => match allowed.get(viewer_id) {
                    Some(known) => *known,
                    None => {
                        let may = self.may_watch(stream, *viewer_id).await?;
                        allowed.insert(*viewer_id, may);
                        may
                    }
                },
            };
            if !may_stay {
                if self.kick_connection(kind, id).await? {
                    cut += 1;
                }
                gone.push(id.clone());
            }
        }
        // Forget the sessions that are gone: the ones just cut and the ones that ended by themselves.
        let reading: std::collections::HashSet<&String> = path.reader_ids.iter().map(|(_, id)| id).collect();
        gone.extend(sessions.keys().filter(|session| !reading.contains(session)).cloned());
        self.forget_sessions(stream.id, &gone).await?;
        Ok(cut)
    }

    /// [`Self::playback`] for the caller of an RPC. A playback token is a
    /// credential for the media server, and those are handed to signed-in
    /// sessions only: a caller with an API token gets the URLs of a stream
    /// anyone may watch, and nothing for one that needs a token.
    pub fn playback_for(&self, stream: &StreamRow, viewer: Option<Uuid>, session: bool) -> Option<Playback> {
        let playback = self.playback(stream, viewer);
        (session || playback.token.is_empty()).then_some(playback)
    }

    /// The URLs a player needs for a stream the caller may play.
    /// `viewer` is who the URLs are for: a restricted stream's token is bound to them.
    pub fn playback(&self, stream: &StreamRow, viewer: Option<Uuid>) -> Playback {
        let cfg = &self.cfg;
        let path = stream.media_path();
        // On air and open to everyone: plain URLs that can be shared with
        // any player. Otherwise the URLs carry a token.
        let restricted = !(stream.is_live() && stream.open_to_everyone());
        let expires = now() + Duration::minutes(PLAYBACK_TOKEN_TTL_MINUTES);
        let token = match viewer.filter(|_| restricted) {
            Some(viewer) => self.playback_token(&stream.short_id, viewer, expires),
            None => String::new(),
        };
        let restricted = !token.is_empty();
        let (first, more) =
            if restricted { (format!("?t={token}"), format!("&t={token}")) } else { (String::new(), String::new()) };
        let dvr = stream.dvr_enabled;
        Playback {
            hls_url: format!("{}/{path}/index.m3u8{first}", cfg.hls_url.trim_end_matches('/')),
            whep_url: format!("{}/{path}/whep{first}", cfg.whep_url.trim_end_matches('/')),
            dvr_list_url: if dvr {
                format!("{}/list?path={path}{more}", cfg.playback_url.trim_end_matches('/'))
            } else {
                String::new()
            },
            dvr_get_url: if dvr {
                format!("{}/get?path={path}{more}", cfg.playback_url.trim_end_matches('/'))
            } else {
                String::new()
            },
            dvr_window_seconds: if dvr { cfg.dvr_window_seconds as i32 } else { 0 },
            token,
            expires_at: if restricted { ts(expires) } else { opt_ts(None) },
            ..Default::default()
        }
    }

    /// Where and with which key a stream's encoder publishes.
    pub fn ingest_info(&self, stream: &StreamRow) -> AppResult<IngestInfo> {
        let cfg = &self.cfg;
        let key = streams::open_stream_key(&self.core, stream)?;
        let name = &stream.short_id;
        let rtmp = cfg.rtmp_url.trim_end_matches('/');
        Ok(IngestInfo {
            stream_name: name.clone(),
            rtmp_server_url: format!("{rtmp}/live"),
            rtmp_stream_key: format!("{name}?token={key}"),
            rtmp_url: format!("{rtmp}/live/{name}?token={key}"),
            srt_url: format!(
                "{}?streamid=publish:live/{name}:{API_USER}:{key}&pkt_size=1316",
                cfg.srt_url.trim_end_matches('/')
            ),
            whip_url: format!("{}/live/{name}/whip", cfg.whip_url.trim_end_matches('/')),
            whip_bearer_token: key.clone(),
            stream_key: key,
            key_rotated_at: ts(stream.key_rotated_at),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_types_map_to_protocols_and_kick_endpoints() {
        assert_eq!(protocol_of("rtmpConn"), "rtmp");
        assert_eq!(protocol_of("srtConn"), "srt");
        assert_eq!(protocol_of("webRTCSession"), "webrtc");
        assert_eq!(protocol_of("hlsSource"), "");
        assert_eq!(kick_endpoint("rtmpConn"), Some("rtmp/conns"));
        assert_eq!(kick_endpoint("webRTCSession"), Some("webrtc/sessions"));
        assert_eq!(kick_endpoint("redirect"), None);
        assert_eq!(kick_endpoint("hlsSession"), Some("hls/sessions"));
    }

    #[test]
    fn the_control_api_answer_is_understood() {
        // As answered by MediaMTX 1.21.4 for one stream with two viewers.
        let body = r#"{"itemCount":1,"pageCount":1,"items":[{"name":"live/abcDEF12345","confName":"~^live/","ready":true,
            "available":true,"online":true,"onlineTime":"2026-11-22T09:33:39.869412656+09:00",
            "source":{"type":"rtmpConn","id":"c9c1d7cc-bc9d-45c4-ba63-82e071bfb7c6"},"tracks":["H264","MPEG-4 Audio"],
            "readers":[{"type":"hlsSession","id":"47a5231d"},{"type":"rtmpConn","id":"d796abc0"}],"inboundBytes":1026793}]}"#;
        let list: PathList = serde_json::from_str(body).unwrap();
        assert_eq!(list.page_count, 1);
        let path = MediaPath::from(list.items.into_iter().next().unwrap());
        assert_eq!(path.name, "live/abcDEF12345");
        assert!(path.online);
        assert_eq!(
            (path.protocol(), path.source_id.as_str(), path.readers),
            ("rtmp", "c9c1d7cc-bc9d-45c4-ba63-82e071bfb7c6", 2)
        );
        // A path without a publisher.
        let idle: PathItem =
            serde_json::from_str(r#"{"name":"live/abcDEF12345","online":false,"source":null,"readers":[]}"#).unwrap();
        let idle = MediaPath::from(idle);
        assert!(!idle.online && idle.source_type.is_empty() && idle.readers == 0);
    }
}
