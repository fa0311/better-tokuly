//! Third-party account connections. Currently Spotify, for "now playing".
//!
//! Tokens are stored encrypted. The integration is off unless Spotify
//! application credentials are configured.
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokuly_core::{keys::random_token, prelude::*};
use tokuly_proto::identity::{Connection, NowPlaying, PinnedSong};

use crate::Identity;

const SPOTIFY: &str = "spotify";
const SPOTIFY_SCOPES: &str = "user-read-currently-playing user-read-playback-state";
const NOW_PLAYING_TTL: Duration = Duration::from_secs(15);

#[derive(Serialize, Deserialize)]
struct PendingConnection {
    user_id: Uuid,
    provider: String,
    redirect_uri: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    scope: Option<String>,
}

/// Cached "nothing playing" marker so idle users do not hit the provider on every profile view.
#[derive(Serialize, Deserialize)]
struct CachedNowPlaying(Option<serde_json::Value>);

fn provider_err(e: impl std::fmt::Display) -> AppError {
    AppError::unavailable("PROVIDER_UNAVAILABLE", format!("music provider request failed: {e}"))
}

/// Maps Spotify's `currently-playing` payload to our message.
pub(crate) fn now_playing_from_spotify(body: &serde_json::Value) -> Option<NowPlaying> {
    let item = body.get("item")?;
    if item.is_null() {
        return None;
    }
    let text = |v: &serde_json::Value, key: &str| v.get(key).and_then(|x| x.as_str()).unwrap_or_default().to_owned();
    let artists = item
        .get("artists")
        .and_then(|a| a.as_array())
        .map(|list| list.iter().filter_map(|a| a.get("name").and_then(|n| n.as_str())).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    let album = item.get("album");
    let artwork_url = album
        .and_then(|a| a.get("images"))
        .and_then(|i| i.as_array())
        .and_then(|images| images.first())
        .map(|img| text(img, "url"))
        .unwrap_or_default();
    Some(NowPlaying {
        provider: SPOTIFY.to_owned(),
        title: text(item, "name"),
        artist: artists,
        album: album.map(|a| text(a, "name")).unwrap_or_default(),
        artwork_url,
        track_url: item.get("external_urls").map(|u| text(u, "spotify")).unwrap_or_default(),
        duration_ms: item.get("duration_ms").and_then(|d| d.as_i64()).unwrap_or(0) as i32,
        progress_ms: body.get("progress_ms").and_then(|d| d.as_i64()).unwrap_or(0) as i32,
        playing: body.get("is_playing").and_then(|p| p.as_bool()).unwrap_or(false),
        updated_at: ts(Utc::now()),
        ..Default::default()
    })
}

/// Whether `redirect_uri` leads back to this instance's web app: the same
/// origin as the public URL (a string prefix would also let
/// `https://tokuly.example.evil.test` through) and below its path.
fn is_own_redirect(public_url: &str, redirect_uri: &str) -> bool {
    let (Ok(ours), Ok(target)) = (url::Url::parse(public_url), url::Url::parse(redirect_uri)) else {
        return false;
    };
    let base = ours.path().trim_end_matches('/');
    let below = target.path().strip_prefix(base).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'));
    ours.origin() == target.origin() && target.username().is_empty() && target.password().is_none() && below
}

impl Identity {
    pub(crate) fn available_providers(&self) -> Vec<String> {
        if self.cfg.spotify_enabled() { vec![SPOTIFY.to_owned()] } else { Vec::new() }
    }

    fn require_provider(&self, provider: &str) -> AppResult<()> {
        ensure!(provider == SPOTIFY, invalid, "PROVIDER_UNKNOWN", "unknown provider");
        ensure!(
            self.cfg.spotify_enabled(),
            unimplemented,
            "PROVIDER_DISABLED",
            "this integration is not configured on this server"
        );
        Ok(())
    }

    pub(crate) async fn connections_of(&self, user_id: Uuid) -> AppResult<Vec<Connection>> {
        let rows: Vec<(String, String, DateTime<Utc>)> = sqlx::query_as(
            "SELECT provider, account_name, connected_at FROM connections WHERE user_id = $1 ORDER BY provider",
        )
        .bind(user_id)
        .fetch_all(&self.core.db)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(provider, account_name, connected_at)| Connection {
                provider,
                account_name,
                connected_at: ts(connected_at),
                ..Default::default()
            })
            .collect())
    }

    pub(crate) async fn begin_provider_connection(
        &self,
        user_id: Uuid,
        provider: &str,
        redirect_uri: &str,
    ) -> AppResult<(String, String)> {
        self.require_provider(provider)?;
        // Only send users back to our own web app.
        ensure!(
            is_own_redirect(self.core.cfg.public_url(), redirect_uri),
            invalid,
            "REDIRECT_INVALID",
            "redirect_uri must point at this Tokuly instance"
        );
        let state = random_token(24);
        self.core
            .cache
            .set_json(
                &format!("id:conn:{state}"),
                &PendingConnection { user_id, provider: provider.to_owned(), redirect_uri: redirect_uri.to_owned() },
                Some(Duration::from_secs(600)),
            )
            .await?;
        let mut url = url::Url::parse("https://accounts.spotify.com/authorize").expect("static url");
        url.query_pairs_mut()
            .append_pair("client_id", &self.cfg.spotify_client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("scope", SPOTIFY_SCOPES)
            .append_pair("state", &state);
        Ok((url.to_string(), state))
    }

    pub(crate) async fn finish_provider_connection(
        &self,
        user_id: Uuid,
        provider: &str,
        code: &str,
        state: &str,
    ) -> AppResult<Connection> {
        self.require_provider(provider)?;
        let pending: PendingConnection =
            self.core.cache.take_json(&format!("id:conn:{state}")).await?.ok_or_else(|| {
                AppError::precondition("CONNECTION_EXPIRED", "the connection request expired, try again")
            })?;
        ensure!(
            pending.user_id == user_id && pending.provider == provider,
            denied,
            "FORBIDDEN",
            "this connection request belongs to another account"
        );
        let tokens: TokenResponse = self
            .core
            .http
            .post("https://accounts.spotify.com/api/token")
            .basic_auth(&self.cfg.spotify_client_id, Some(&self.cfg.spotify_client_secret))
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", pending.redirect_uri.as_str()),
            ])
            .send()
            .await
            .map_err(provider_err)?
            .error_for_status()
            .map_err(provider_err)?
            .json()
            .await
            .map_err(provider_err)?;
        let profile: serde_json::Value = self
            .core
            .http
            .get(format!("{}/v1/me", self.cfg.spotify_api_url.trim_end_matches('/')))
            .bearer_auth(&tokens.access_token)
            .send()
            .await
            .map_err(provider_err)?
            .error_for_status()
            .map_err(provider_err)?
            .json()
            .await
            .map_err(provider_err)?;
        let account_id = profile.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_owned();
        let account_name = profile
            .get("display_name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(&account_id)
            .to_owned();
        let keys = &self.core.keys;
        let connected_at: DateTime<Utc> = sqlx::query_scalar(
            "INSERT INTO connections (user_id, provider, account_id, account_name, access_token, refresh_token, expires_at, scopes)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (user_id, provider) DO UPDATE SET
                account_id = EXCLUDED.account_id, account_name = EXCLUDED.account_name, access_token = EXCLUDED.access_token,
                refresh_token = COALESCE(EXCLUDED.refresh_token, connections.refresh_token), expires_at = EXCLUDED.expires_at,
                scopes = EXCLUDED.scopes, connected_at = now()
             RETURNING connected_at",
        )
        .bind(user_id)
        .bind(provider)
        .bind(&account_id)
        .bind(&account_name)
        .bind(keys.seal("connection", tokens.access_token.as_bytes()))
        .bind(tokens.refresh_token.as_ref().map(|t| keys.seal("connection", t.as_bytes())))
        .bind(tokens.expires_in.map(|s| Utc::now() + chrono::Duration::seconds(s)))
        .bind(tokens.scope.unwrap_or_default())
        .fetch_one(&self.core.db)
        .await?;
        Ok(Connection {
            provider: provider.to_owned(),
            account_name,
            connected_at: ts(connected_at),
            ..Default::default()
        })
    }

    /// A valid Spotify access token for the user, refreshing it when expired.
    pub(crate) async fn spotify_access_token(&self, user_id: Uuid) -> AppResult<Option<String>> {
        let row: Option<(Vec<u8>, Option<Vec<u8>>, Option<DateTime<Utc>>)> = sqlx::query_as(
            "SELECT access_token, refresh_token, expires_at FROM connections WHERE user_id = $1 AND provider = $2",
        )
        .bind(user_id)
        .bind(SPOTIFY)
        .fetch_optional(&self.core.db)
        .await?;
        let Some((access, refresh, expires_at)) = row else {
            return Ok(None);
        };
        let keys = &self.core.keys;
        let open = |sealed: &[u8]| keys.open("connection", sealed).and_then(|b| String::from_utf8(b).ok());
        if expires_at.is_none_or(|t| t > Utc::now() + chrono::Duration::seconds(30)) {
            return Ok(open(&access));
        }
        let Some(refresh_token) = refresh.as_deref().and_then(open) else {
            return Ok(None);
        };
        let tokens: TokenResponse = self
            .core
            .http
            .post("https://accounts.spotify.com/api/token")
            .basic_auth(&self.cfg.spotify_client_id, Some(&self.cfg.spotify_client_secret))
            .form(&[("grant_type", "refresh_token"), ("refresh_token", refresh_token.as_str())])
            .send()
            .await
            .map_err(provider_err)?
            .error_for_status()
            .map_err(provider_err)?
            .json()
            .await
            .map_err(provider_err)?;
        sqlx::query(
            "UPDATE connections SET access_token = $3, refresh_token = COALESCE($4, refresh_token), expires_at = $5 WHERE user_id = $1 AND provider = $2",
        )
        .bind(user_id)
        .bind(SPOTIFY)
        .bind(keys.seal("connection", tokens.access_token.as_bytes()))
        .bind(tokens.refresh_token.as_ref().map(|t| keys.seal("connection", t.as_bytes())))
        .bind(tokens.expires_in.map(|s| Utc::now() + chrono::Duration::seconds(s)))
        .execute(&self.core.db)
        .await?;
        Ok(Some(tokens.access_token))
    }

    /// What the user is listening to right now, if they connected a music
    /// service. Cached briefly; provider failures yield `None` rather than an error.
    pub async fn now_playing(&self, user_id: Uuid) -> Option<NowPlaying> {
        if !self.cfg.spotify_enabled() {
            return None;
        }
        let cache_key = format!("id:np:{user_id}");
        if let Ok(Some(CachedNowPlaying(cached))) = self.core.cache.get_json::<CachedNowPlaying>(&cache_key).await {
            return cached.and_then(|v| serde_json::from_value(v).ok());
        }
        let fetched: AppResult<Option<NowPlaying>> = async {
            let Some(token) = self.spotify_access_token(user_id).await? else {
                return Ok(None);
            };
            let response = self
                .core
                .http
                .get(format!("{}/v1/me/player/currently-playing", self.cfg.spotify_api_url.trim_end_matches('/')))
                .bearer_auth(token)
                .timeout(Duration::from_secs(4))
                .send()
                .await
                .map_err(provider_err)?;
            if response.status() == reqwest::StatusCode::NO_CONTENT {
                return Ok(None);
            }
            let body: serde_json::Value =
                response.error_for_status().map_err(provider_err)?.json().await.map_err(provider_err)?;
            Ok(now_playing_from_spotify(&body))
        }
        .await;
        let now_playing = match fetched {
            Ok(np) => np,
            Err(e) => {
                tracing::debug!(%user_id, error = %e, "now playing lookup failed");
                None
            }
        };
        let cached = CachedNowPlaying(now_playing.as_ref().and_then(|np| serde_json::to_value(np).ok()));
        let _ = self.core.cache.set_json(&cache_key, &cached, Some(NOW_PLAYING_TTL)).await;
        now_playing
    }
}

/// The Spotify track ID in a track URL (`https://open.spotify.com/track/…`,
/// with or without a locale segment), URI (`spotify:track:…`) or bare ID.
pub(crate) fn spotify_track_id(input: &str) -> Option<String> {
    let input = input.trim();
    let candidate = if let Some(rest) = input.strip_prefix("spotify:track:") {
        rest.to_owned()
    } else if input.contains("://") {
        let url = url::Url::parse(input).ok()?;
        if url.scheme() != "https" || url.host_str() != Some("open.spotify.com") {
            return None;
        }
        let segments: Vec<&str> = url.path_segments()?.collect();
        let at = segments.iter().position(|s| *s == "track")?;
        (*segments.get(at + 1)?).to_owned()
    } else {
        input.to_owned()
    };
    (candidate.len() == 22 && candidate.bytes().all(|b| b.is_ascii_alphanumeric())).then_some(candidate)
}

/// Maps a Spotify track object to a pinned song.
pub(crate) fn pinned_song_from_spotify(track: &serde_json::Value) -> Option<PinnedSong> {
    let text = |v: &serde_json::Value, key: &str| v.get(key).and_then(|x| x.as_str()).unwrap_or_default().to_owned();
    let track_id = text(track, "id");
    if track_id.is_empty() {
        return None;
    }
    let artist = track
        .get("artists")
        .and_then(|a| a.as_array())
        .map(|artists| artists.iter().map(|a| text(a, "name")).filter(|n| !n.is_empty()).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    let album = track.get("album").cloned().unwrap_or_default();
    let artwork_url = album
        .get("images")
        .and_then(|i| i.as_array())
        .and_then(|images| images.first())
        .map(|image| text(image, "url"))
        .unwrap_or_default();
    let track_url = track.pointer("/external_urls/spotify").and_then(|u| u.as_str()).unwrap_or_default().to_owned();
    Some(PinnedSong {
        provider: SPOTIFY.to_owned(),
        title: text(track, "name"),
        artist,
        album: text(&album, "name"),
        // Only Spotify's own pages and images are kept.
        artwork_url: if artwork_url.starts_with("https://") { artwork_url } else { String::new() },
        track_url: if track_url.starts_with("https://open.spotify.com/") {
            track_url
        } else {
            format!("https://open.spotify.com/track/{track_id}")
        },
        track_id,
        pinned_at: ts(Utc::now()),
        ..Default::default()
    })
}

impl Identity {
    /// The song to pin: the track named (`track`), or what the account's
    /// Spotify is playing now (`track` empty).
    pub(crate) async fn lookup_song(&self, user_id: Uuid, track: &str) -> AppResult<PinnedSong> {
        let not_connected = || AppError::precondition("SPOTIFY_NOT_CONNECTED", "connect Spotify first");
        if !self.cfg.spotify_enabled() {
            return Err(not_connected());
        }
        let track_id = if track.trim().is_empty() {
            None
        } else {
            Some(
                spotify_track_id(track)
                    .ok_or_else(|| AppError::invalid("TRACK_INVALID", "paste a Spotify track link"))?,
            )
        };
        let token = self.spotify_access_token(user_id).await?.ok_or_else(not_connected)?;
        let api = self.cfg.spotify_api_url.trim_end_matches('/');
        let url = match &track_id {
            Some(id) => format!("{api}/v1/tracks/{id}"),
            None => format!("{api}/v1/me/player/currently-playing"),
        };
        let response = self
            .core
            .http
            .get(url)
            .bearer_auth(token)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(provider_err)?;
        if track_id.is_none() && response.status() == reqwest::StatusCode::NO_CONTENT {
            return Err(AppError::precondition("NOTHING_PLAYING", "nothing is playing right now"));
        }
        if track_id.is_some()
            && matches!(response.status(), reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::BAD_REQUEST)
        {
            return Err(AppError::invalid("TRACK_INVALID", "this track does not exist"));
        }
        let body: serde_json::Value =
            response.error_for_status().map_err(provider_err)?.json().await.map_err(provider_err)?;
        let item = if track_id.is_some() { Some(&body) } else { body.get("item").filter(|item| !item.is_null()) };
        item.and_then(pinned_song_from_spotify)
            .ok_or_else(|| AppError::precondition("NOTHING_PLAYING", "nothing is playing right now"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_ids_come_from_links_uris_and_ids() {
        let id = "4uLU6hMCjMI75M1A2tKUQC";
        assert_eq!(spotify_track_id(&format!("https://open.spotify.com/track/{id}?si=abc")).as_deref(), Some(id));
        assert_eq!(spotify_track_id(&format!("https://open.spotify.com/intl-ja/track/{id}")).as_deref(), Some(id));
        assert_eq!(spotify_track_id(&format!("spotify:track:{id}")).as_deref(), Some(id));
        assert_eq!(spotify_track_id(id).as_deref(), Some(id));
        assert_eq!(spotify_track_id(&format!("https://evil.example/track/{id}")), None);
        assert_eq!(spotify_track_id("https://open.spotify.com/album/4uLU6hMCjMI75M1A2tKUQC"), None);
        assert_eq!(spotify_track_id("short"), None);
    }

    #[test]
    fn maps_a_track_to_a_pinned_song() {
        let track = serde_json::json!({
            "id": "4uLU6hMCjMI75M1A2tKUQC", "name": "晴る",
            "artists": [{ "name": "ヨルシカ" }],
            "album": { "name": "晴る", "images": [{ "url": "https://i.scdn.co/image/x" }, { "url": "https://i.scdn.co/image/y" }] },
            "external_urls": { "spotify": "javascript:alert(1)" }
        });
        let song = pinned_song_from_spotify(&track).unwrap();
        assert_eq!((song.title.as_str(), song.artist.as_str()), ("晴る", "ヨルシカ"));
        assert_eq!(song.artwork_url, "https://i.scdn.co/image/x");
        assert_eq!(song.track_url, "https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC");
        assert!(pinned_song_from_spotify(&serde_json::json!({})).is_none());
    }

    #[test]
    fn maps_spotify_payload() {
        let body = serde_json::json!({
            "is_playing": true,
            "progress_ms": 42000,
            "item": {
                "name": "ライラック",
                "duration_ms": 247000,
                "artists": [{ "name": "Mrs. GREEN APPLE" }, { "name": "Guest" }],
                "album": { "name": "ライラック", "images": [{ "url": "https://i.scdn.co/image/abc" }, { "url": "small" }] },
                "external_urls": { "spotify": "https://open.spotify.com/track/xyz" }
            }
        });
        let np = now_playing_from_spotify(&body).unwrap();
        assert_eq!(np.title, "ライラック");
        assert_eq!(np.artist, "Mrs. GREEN APPLE, Guest");
        assert_eq!(np.artwork_url, "https://i.scdn.co/image/abc");
        assert_eq!((np.duration_ms, np.progress_ms, np.playing), (247000, 42000, true));
        assert_eq!(np.track_url, "https://open.spotify.com/track/xyz");
    }

    #[test]
    fn only_our_own_web_app_is_a_redirect_target() {
        let ours = "https://tokuly.example";
        assert!(is_own_redirect(ours, "https://tokuly.example/settings/connections/callback"));
        assert!(is_own_redirect(ours, "https://tokuly.example"));
        for theirs in [
            "https://tokuly.example.evil.test/callback",
            "https://tokuly.example@evil.test/callback",
            "https://tokuly.examplex/callback",
            "http://tokuly.example/callback",
            "https://tokuly.example:8443/callback",
            "not a url",
        ] {
            assert!(!is_own_redirect(ours, theirs), "{theirs}");
        }
    }

    #[test]
    fn nothing_playing_maps_to_none() {
        assert!(now_playing_from_spotify(&serde_json::json!({ "is_playing": false, "item": null })).is_none());
        assert!(now_playing_from_spotify(&serde_json::json!({})).is_none());
    }
}
