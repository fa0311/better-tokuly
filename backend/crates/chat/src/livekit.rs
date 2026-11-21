//! LiveKit: access tokens and the handful of room-service calls the chat
//! domain needs.
//!
//! Tokens are minted here rather than through LiveKit's SDK: an access
//! token is an HS256 JWT signed with the API secret, with the API key as
//! issuer, the participant identity as subject and a `video` grant saying
//! what the holder may do. Clients connect to the public URL with a join
//! token; the server talks to LiveKit's Twirp API (JSON over HTTP) with
//! short-lived tokens carrying the admin grants a call needs.
use std::time::Duration;

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokuly_core::prelude::*;

use crate::ChatConfig;

/// How long a join token can be used to connect. Once connected, LiveKit
/// keeps the session alive by itself.
pub const JOIN_TOKEN_TTL: Duration = Duration::from_secs(600);
/// Lifetime of the tokens the server signs for its own API calls.
const API_TOKEN_TTL: Duration = Duration::from_secs(60);
/// A LiveKit API call that takes longer than this has failed.
const API_TIMEOUT: Duration = Duration::from_secs(5);
/// Seconds LiveKit keeps a room nobody has joined yet.
const EMPTY_ROOM_TIMEOUT_SECONDS: u32 = 300;

fn is_false(value: &bool) -> bool {
    !*value
}

/// The `video` claim of a LiveKit access token.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoGrant {
    /// The room the grant applies to.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub room: String,
    /// May join `room` as a participant.
    #[serde(default, skip_serializing_if = "is_false")]
    pub room_join: bool,
    /// May create and delete rooms.
    #[serde(default, skip_serializing_if = "is_false")]
    pub room_create: bool,
    /// May list rooms.
    #[serde(default, skip_serializing_if = "is_false")]
    pub room_list: bool,
    /// May administer `room` (list and remove participants).
    #[serde(default, skip_serializing_if = "is_false")]
    pub room_admin: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_publish: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_subscribe: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_publish_data: Option<bool>,
}

/// The claims of a LiveKit access token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    /// The API key.
    pub iss: String,
    /// The participant identity (the user ID); empty for API tokens.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sub: String,
    pub nbf: u64,
    pub exp: u64,
    /// Display name of the participant.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub video: VideoGrant,
}

/// A failed LiveKit API call.
#[derive(Debug)]
pub enum LiveKitError {
    /// LiveKit answered with a Twirp error (`code` as in the Twirp spec:
    /// `not_found`, `unauthenticated`, `permission_denied`…).
    Api { code: String, message: String },
    /// LiveKit could not be reached or answered nonsense.
    Transport(String),
}

impl std::fmt::Display for LiveKitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Api { code, message } => write!(f, "LiveKit error {code}: {message}"),
            Self::Transport(message) => write!(f, "LiveKit unreachable: {message}"),
        }
    }
}

impl LiveKitError {
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::Api { code, .. } if code == "not_found")
    }
}

#[derive(Deserialize)]
struct TwirpError {
    #[serde(default)]
    code: String,
    #[serde(default)]
    msg: String,
}

#[derive(Debug, Deserialize)]
pub struct Room {
    #[serde(default)]
    pub sid: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Deserialize)]
struct ListRoomsResponse {
    #[serde(default)]
    rooms: Vec<Room>,
}

#[derive(Deserialize)]
struct Participant {
    #[serde(default)]
    identity: String,
}

#[derive(Deserialize)]
struct ListParticipantsResponse {
    #[serde(default)]
    participants: Vec<Participant>,
}

/// A configured LiveKit deployment.
pub struct LiveKit {
    http: reqwest::Client,
    api_url: String,
    public_url: String,
    api_key: String,
    api_secret: String,
}

impl LiveKit {
    /// `None` when calls are not configured.
    pub fn from_config(cfg: &ChatConfig) -> anyhow::Result<Option<Self>> {
        if !cfg.calls_enabled() {
            return Ok(None);
        }
        let http = reqwest::Client::builder()
            .user_agent("Tokuly/1.0")
            .timeout(API_TIMEOUT)
            .build()
            .map_err(|e| anyhow::anyhow!("LiveKit http client: {e}"))?;
        Ok(Some(Self {
            http,
            api_url: cfg.livekit_url.trim_end_matches('/').to_owned(),
            public_url: cfg.livekit_public_url.clone(),
            api_key: cfg.livekit_api_key.clone(),
            api_secret: cfg.livekit_api_secret.clone(),
        }))
    }

    /// The URL clients connect to.
    pub fn public_url(&self) -> &str {
        &self.public_url
    }

    fn sign(&self, identity: &str, name: &str, video: VideoGrant, ttl: Duration) -> AppResult<String> {
        let now = jsonwebtoken::get_current_timestamp();
        let claims = Claims {
            iss: self.api_key.clone(),
            sub: identity.to_owned(),
            // A little slack for clocks that disagree.
            nbf: now.saturating_sub(10),
            exp: now + ttl.as_secs(),
            name: name.to_owned(),
            video,
        };
        let key = EncodingKey::from_secret(self.api_secret.as_bytes()).map_err(|e| AppError::internal(anyhow::anyhow!("LiveKit signing key: {e}")))?;
        jsonwebtoken::encode(&Header::new(Algorithm::HS256), &claims, &key).map_err(|e| AppError::internal(anyhow::anyhow!("LiveKit token: {e}")))
    }

    /// A token that lets `identity` join `room`, publish and subscribe.
    pub fn join_token(&self, room: &str, identity: &str, name: &str) -> AppResult<String> {
        let grant = VideoGrant {
            room: room.to_owned(),
            room_join: true,
            can_publish: Some(true),
            can_subscribe: Some(true),
            can_publish_data: Some(true),
            ..Default::default()
        };
        self.sign(identity, name, grant, JOIN_TOKEN_TTL)
    }

    /// A token for one of the server's own API calls.
    pub fn api_token(&self, grant: VideoGrant) -> AppResult<String> {
        self.sign("", "", grant, API_TOKEN_TTL)
    }

    async fn call<T: DeserializeOwned>(&self, method: &str, grant: VideoGrant, body: serde_json::Value) -> Result<T, LiveKitError> {
        let token = self.api_token(grant).map_err(|e| LiveKitError::Transport(e.to_string()))?;
        let response = self
            .http
            .post(format!("{}/twirp/livekit.RoomService/{method}", self.api_url))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(|e| LiveKitError::Transport(e.to_string()))?;
        let status = response.status();
        let bytes = response.bytes().await.map_err(|e| LiveKitError::Transport(e.to_string()))?;
        if !status.is_success() {
            return Err(match serde_json::from_slice::<TwirpError>(&bytes) {
                Ok(error) if !error.code.is_empty() => LiveKitError::Api { code: error.code, message: error.msg },
                _ => LiveKitError::Transport(format!("HTTP {status}")),
            });
        }
        serde_json::from_slice(&bytes).map_err(|e| LiveKitError::Transport(format!("undecodable answer to {method}: {e}")))
    }

    /// Creates a room (or returns the existing one of that name).
    pub async fn create_room(&self, room: &str, max_participants: u32) -> Result<Room, LiveKitError> {
        let grant = VideoGrant { room_create: true, ..Default::default() };
        let body = serde_json::json!({ "name": room, "empty_timeout": EMPTY_ROOM_TIMEOUT_SECONDS, "max_participants": max_participants });
        self.call("CreateRoom", grant, body).await
    }

    /// Names of the rooms that exist, optionally only among `names`.
    pub async fn list_rooms(&self, names: &[String]) -> Result<Vec<String>, LiveKitError> {
        let grant = VideoGrant { room_list: true, ..Default::default() };
        let response: ListRoomsResponse = self.call("ListRooms", grant, serde_json::json!({ "names": names })).await?;
        Ok(response.rooms.into_iter().map(|room| room.name).collect())
    }

    /// Identities connected to a room; `None` when the room does not exist.
    pub async fn list_participants(&self, room: &str) -> Result<Option<Vec<String>>, LiveKitError> {
        let grant = VideoGrant { room: room.to_owned(), room_admin: true, ..Default::default() };
        match self.call::<ListParticipantsResponse>("ListParticipants", grant, serde_json::json!({ "room": room })).await {
            Ok(response) => Ok(Some(response.participants.into_iter().map(|participant| participant.identity).collect())),
            Err(e) if e.is_not_found() => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Closes a room, disconnecting everyone in it. A room that is already
    /// gone is not an error.
    pub async fn delete_room(&self, room: &str) -> Result<(), LiveKitError> {
        let grant = VideoGrant { room_create: true, ..Default::default() };
        match self.call::<serde_json::Value>("DeleteRoom", grant, serde_json::json!({ "room": room })).await {
            Ok(_) => Ok(()),
            Err(e) if e.is_not_found() => Ok(()),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use jsonwebtoken::{DecodingKey, Validation};

    use super::*;

    fn livekit() -> LiveKit {
        let cfg = ChatConfig {
            livekit_url: "http://127.0.0.1:7880/".into(),
            livekit_public_url: "ws://127.0.0.1:7880".into(),
            livekit_api_key: "devkey".into(),
            livekit_api_secret: "a-secret-of-at-least-thirty-two-characters".into(),
            ..Default::default()
        };
        LiveKit::from_config(&cfg).unwrap().expect("calls are configured")
    }

    fn decode(token: &str, secret: &str) -> jsonwebtoken::errors::Result<Claims> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&["devkey"]);
        validation.validate_nbf = true;
        validation.set_required_spec_claims(&["exp", "iss", "nbf"]);
        jsonwebtoken::decode::<Claims>(token, &DecodingKey::from_secret(secret.as_bytes())?, &validation).map(|data| data.claims)
    }

    #[test]
    fn calls_need_configuration() {
        assert!(LiveKit::from_config(&ChatConfig::default()).unwrap().is_none());
        assert_eq!(livekit().api_url, "http://127.0.0.1:7880");
    }

    #[test]
    fn join_tokens_carry_identity_and_the_room_grant() {
        let livekit = livekit();
        let token = livekit.join_token("call-123", "user-1", "たろう").unwrap();
        assert_eq!(jsonwebtoken::decode_header(&token).unwrap().alg, Algorithm::HS256);
        let claims = decode(&token, "a-secret-of-at-least-thirty-two-characters").unwrap();
        assert_eq!(claims.iss, "devkey");
        assert_eq!(claims.sub, "user-1");
        assert_eq!(claims.name, "たろう");
        assert_eq!(claims.video.room, "call-123");
        assert!(claims.video.room_join);
        assert_eq!(claims.video.can_publish, Some(true));
        assert_eq!(claims.video.can_subscribe, Some(true));
        assert!(!claims.video.room_admin && !claims.video.room_create && !claims.video.room_list);
        let lifetime = claims.exp - jsonwebtoken::get_current_timestamp();
        assert!((JOIN_TOKEN_TTL.as_secs() - 5..=JOIN_TOKEN_TTL.as_secs()).contains(&lifetime), "expires in ten minutes, got {lifetime}s");
    }

    #[test]
    fn tokens_are_signed_with_the_api_secret() {
        let token = livekit().join_token("call-123", "user-1", "").unwrap();
        assert!(decode(&token, "another-secret-of-at-least-thirty-two-chars").is_err());
    }

    #[test]
    fn the_video_grant_uses_livekit_claim_names() {
        let livekit = livekit();
        let token = livekit.api_token(VideoGrant { room: "r".into(), room_admin: true, ..Default::default() }).unwrap();
        // The raw payload, as LiveKit reads it.
        let payload = token.split('.').nth(1).unwrap();
        let claims: serde_json::Value = jsonwebtoken::dangerous::insecure_decode::<serde_json::Value>(&token).unwrap().claims;
        assert!(!payload.is_empty());
        assert_eq!(claims["video"], serde_json::json!({ "room": "r", "roomAdmin": true }));
        assert!(claims.get("sub").is_none(), "API tokens have no participant identity");
        assert!(claims.get("name").is_none());
    }
}
