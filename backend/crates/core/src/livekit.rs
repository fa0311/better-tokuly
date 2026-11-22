//! LiveKit: access tokens, the room service and webhook verification.
//!
//! Voice and video run on a LiveKit server next to the API. Three things
//! happen here, none of them through LiveKit's SDK:
//!
//! - **Access tokens** ([`LiveKit::token`]). A client connects to the public
//!   URL with a token saying who it is and what it may do in one room: an
//!   HS256 JWT signed with the API secret, with the API key as issuer, the
//!   participant identity as subject and a `video` grant.
//! - **The room service** (`create_room`, `list_participants`,
//!   `update_participant`…): LiveKit's Twirp API, JSON over HTTP, each call
//!   authorised by a short-lived token carrying the grant it needs.
//! - **Webhooks** ([`LiveKit::verify_webhook`]): LiveKit signs what it posts
//!   with the same key pair.
//!
//! [`Core::livekit`](crate::CoreInner::livekit) always exists. Without the
//! four `TOKULY_LIVEKIT_*` settings it is *not configured*: every method
//! fails with `unavailable` / [`NOT_CONFIGURED`], so a domain can keep its
//! other features working and say clearly that audio is off
//! ([`LiveKit::is_configured`] tells in advance).
//!
//! Failures of a configured deployment are `unavailable` too:
//! [`UNAVAILABLE`] when LiveKit cannot be reached or answers with a server
//! error, [`REJECTED`] when it refuses the request (a wrong API secret, a
//! malformed request). "Not there" is not a failure: methods that address
//! a room or a participant say so in their return value.
use std::{borrow::Cow, sync::Arc, time::Duration};

use base64::Engine as _;
use connectrpc::ErrorCode;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;

use crate::{
    config::Config,
    error::{AppError, AppResult},
};

/// Reason of every error while LiveKit is not configured.
pub const NOT_CONFIGURED: &str = "LIVEKIT_NOT_CONFIGURED";
/// Reason when LiveKit cannot be reached or fails on its side.
pub const UNAVAILABLE: &str = "LIVEKIT_UNAVAILABLE";
/// Reason when LiveKit refuses a request.
pub const REJECTED: &str = "LIVEKIT_REJECTED";
/// Reason when a webhook is not signed by this deployment's key pair.
pub const WEBHOOK_UNAUTHENTICATED: &str = "LIVEKIT_WEBHOOK_UNAUTHENTICATED";
/// Reason when a signed webhook carries a body that is not a webhook event.
pub const WEBHOOK_INVALID: &str = "LIVEKIT_WEBHOOK_INVALID";

/// How long a join token can be used to connect when nothing else is asked
/// for. Once connected, LiveKit keeps the session alive by itself.
pub const DEFAULT_TOKEN_TTL: Duration = Duration::from_secs(600);
/// Lifetime of the tokens signed for the server's own API calls.
const API_TOKEN_TTL: Duration = Duration::from_secs(60);
/// A room-service call that takes longer than this has failed.
const API_TIMEOUT: Duration = Duration::from_secs(5);
/// Clocks that disagree by this much still accept each other's tokens.
const CLOCK_SLACK_SECONDS: u64 = 10;

// ───────────────────────────── tokens ─────────────────────────────

/// Where a published track comes from. A token or a permission can limit a
/// participant to some sources (an audio room: the microphone only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TrackSource {
    Camera,
    Microphone,
    ScreenShare,
    ScreenShareAudio,
    #[default]
    #[serde(other)]
    Unknown,
}

impl TrackSource {
    /// The name access tokens use (`canPublishSources`).
    pub fn grant_name(self) -> &'static str {
        match self {
            Self::Camera => "camera",
            Self::Microphone => "microphone",
            Self::ScreenShare => "screen_share",
            Self::ScreenShareAudio => "screen_share_audio",
            Self::Unknown => "unknown",
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// The `video` claim of a LiveKit access token, under LiveKit's own names.
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
    /// May administer `room`: list, update and remove participants, mute
    /// tracks, send data.
    #[serde(default, skip_serializing_if = "is_false")]
    pub room_admin: bool,
    /// LiveKit grants what a token leaves out, so join tokens always say
    /// all three.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_publish: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_subscribe: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_publish_data: Option<bool>,
    /// Sources the holder may publish ([`TrackSource::grant_name`]); empty
    /// means any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub can_publish_sources: Vec<String>,
    /// Present in the room without being shown to the others.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
}

/// The claims of a LiveKit access token.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    /// The API key.
    pub iss: String,
    /// The participant identity; empty for the server's API tokens and for
    /// webhooks.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sub: String,
    pub nbf: u64,
    pub exp: u64,
    /// Display name of the participant.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Free-form participant metadata, visible to everyone in the room.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub metadata: String,
    /// Webhooks only: base64 SHA-256 of the request body.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sha256: String,
    #[serde(default)]
    pub video: VideoGrant,
}

/// What a join token says about its holder. Start from
/// [`TokenOptions::join`] and narrow it down:
///
/// ```ignore
/// let listener = TokenOptions::join(&room, &user_id.to_string())
///     .name(&display_name)
///     .publish(false)
///     .publish_data(false);
/// let token = core.livekit.token(&listener)?;
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenOptions {
    pub room: String,
    /// Unique per participant in a room: a second connection with the same
    /// identity replaces the first.
    pub identity: String,
    pub name: String,
    pub metadata: String,
    pub ttl: Duration,
    /// May join the room.
    pub join: bool,
    pub can_publish: bool,
    pub can_subscribe: bool,
    pub can_publish_data: bool,
    /// Sources the holder may publish; empty means any.
    pub publish_sources: Vec<TrackSource>,
    /// May administer the room (moderation from a client).
    pub admin: bool,
    /// Joins unseen (recorders, moderators listening in).
    pub hidden: bool,
}

impl TokenOptions {
    /// A participant who may join `room`, publish anything and subscribe,
    /// for [`DEFAULT_TOKEN_TTL`].
    pub fn join(room: &str, identity: &str) -> Self {
        Self {
            room: room.to_owned(),
            identity: identity.to_owned(),
            name: String::new(),
            metadata: String::new(),
            ttl: DEFAULT_TOKEN_TTL,
            join: true,
            can_publish: true,
            can_subscribe: true,
            can_publish_data: true,
            publish_sources: Vec::new(),
            admin: false,
            hidden: false,
        }
    }

    pub fn name(mut self, name: &str) -> Self {
        self.name = name.to_owned();
        self
    }

    pub fn metadata(mut self, metadata: impl Into<String>) -> Self {
        self.metadata = metadata.into();
        self
    }

    pub fn ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    pub fn publish(mut self, allowed: bool) -> Self {
        self.can_publish = allowed;
        self
    }

    pub fn subscribe(mut self, allowed: bool) -> Self {
        self.can_subscribe = allowed;
        self
    }

    pub fn publish_data(mut self, allowed: bool) -> Self {
        self.can_publish_data = allowed;
        self
    }

    /// Limits what may be published to these sources.
    pub fn sources(mut self, sources: &[TrackSource]) -> Self {
        self.publish_sources = sources.to_vec();
        self
    }

    pub fn admin(mut self, admin: bool) -> Self {
        self.admin = admin;
        self
    }

    pub fn hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }

    fn grant(&self) -> VideoGrant {
        VideoGrant {
            room: self.room.clone(),
            room_join: self.join,
            room_admin: self.admin,
            can_publish: Some(self.can_publish),
            can_subscribe: Some(self.can_subscribe),
            can_publish_data: Some(self.can_publish_data),
            can_publish_sources: self.publish_sources.iter().map(|source| source.grant_name().to_owned()).collect(),
            hidden: self.hidden,
            ..Default::default()
        }
    }
}

// ───────────────────────────── room service types ─────────────────────────────

/// Integers arrive as numbers or, for 64-bit fields, as strings (protobuf
/// JSON); `null` and absence mean zero.
fn lenient_i64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Number(i64),
        Text(String),
    }
    match Option::<Raw>::deserialize(deserializer)? {
        None => Ok(0),
        Some(Raw::Number(value)) => Ok(value),
        Some(Raw::Text(text)) => text.parse().map_err(serde::de::Error::custom),
    }
}

/// `null` where a list or a text is expected means empty.
fn null_as_default<'de, D: Deserializer<'de>, T: Default + Deserialize<'de>>(deserializer: D) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// A room as LiveKit reports it. The room service answers with protobuf
/// field names, webhooks with their camelCase form; both are understood.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Room {
    #[serde(default)]
    pub sid: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub metadata: String,
    /// Seconds the room is kept while nobody has joined it yet.
    #[serde(default, alias = "emptyTimeout")]
    pub empty_timeout: u32,
    /// Seconds the room is kept after the last participant left.
    #[serde(default, alias = "departureTimeout")]
    pub departure_timeout: u32,
    /// 0 = no limit.
    #[serde(default, alias = "maxParticipants")]
    pub max_participants: u32,
    /// Unix seconds.
    #[serde(default, alias = "creationTime", deserialize_with = "lenient_i64")]
    pub creation_time: i64,
    #[serde(default, alias = "numParticipants")]
    pub num_participants: u32,
    #[serde(default, alias = "numPublishers")]
    pub num_publishers: u32,
}

/// How far a participant's connection has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ParticipantState {
    /// Signalling is connected.
    #[default]
    Joining,
    /// Negotiated; media is not flowing yet.
    Joined,
    /// Media is flowing.
    Active,
    Disconnected,
    #[serde(other)]
    Unknown,
}

/// What a participant may do in a room. An update replaces all of it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permission {
    #[serde(default, alias = "canSubscribe")]
    pub can_subscribe: bool,
    #[serde(default, alias = "canPublish")]
    pub can_publish: bool,
    #[serde(default, alias = "canPublishData")]
    pub can_publish_data: bool,
    /// Sources the participant may publish; empty means any.
    #[serde(default, alias = "canPublishSources", deserialize_with = "null_as_default")]
    pub can_publish_sources: Vec<TrackSource>,
    #[serde(default)]
    pub hidden: bool,
    /// May change their own name and metadata.
    #[serde(default, alias = "canUpdateMetadata")]
    pub can_update_metadata: bool,
}

impl Permission {
    /// The permission a join token made with `options` starts with.
    pub fn of(options: &TokenOptions) -> Self {
        Self {
            can_subscribe: options.can_subscribe,
            can_publish: options.can_publish,
            can_publish_data: options.can_publish_data,
            can_publish_sources: options.publish_sources.clone(),
            hidden: options.hidden,
            can_update_metadata: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TrackKind {
    #[default]
    Audio,
    Video,
    Data,
    #[serde(other)]
    Unknown,
}

/// A track a participant publishes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Track {
    #[serde(default)]
    pub sid: String,
    #[serde(default, rename = "type")]
    pub kind: TrackKind,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub source: TrackSource,
}

/// Someone connected to a room.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Participant {
    #[serde(default)]
    pub sid: String,
    #[serde(default)]
    pub identity: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub state: ParticipantState,
    #[serde(default, deserialize_with = "null_as_default")]
    pub metadata: String,
    /// Unix seconds.
    #[serde(default, alias = "joinedAt", deserialize_with = "lenient_i64")]
    pub joined_at: i64,
    #[serde(default)]
    pub permission: Option<Permission>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub tracks: Vec<Track>,
    #[serde(default, alias = "isPublisher")]
    pub is_publisher: bool,
}

/// Settings of a room to create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomOptions {
    pub name: String,
    /// Seconds LiveKit keeps the room while nobody has joined it yet.
    pub empty_timeout_seconds: u32,
    /// Seconds LiveKit keeps the room after the last participant left.
    pub departure_timeout_seconds: u32,
    /// 0 = no limit.
    pub max_participants: u32,
    pub metadata: String,
}

impl RoomOptions {
    /// A room without a participant limit that waits five minutes for its
    /// first participant and twenty seconds after its last.
    pub fn named(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            empty_timeout_seconds: 300,
            departure_timeout_seconds: 20,
            max_participants: 0,
            metadata: String::new(),
        }
    }

    pub fn empty_timeout(mut self, seconds: u32) -> Self {
        self.empty_timeout_seconds = seconds;
        self
    }

    pub fn departure_timeout(mut self, seconds: u32) -> Self {
        self.departure_timeout_seconds = seconds;
        self
    }

    pub fn max_participants(mut self, max: u32) -> Self {
        self.max_participants = max;
        self
    }

    pub fn metadata(mut self, metadata: impl Into<String>) -> Self {
        self.metadata = metadata.into();
        self
    }
}

/// A change to a connected participant; `None` leaves that part alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParticipantUpdate {
    /// Replaces the whole permission, effective at once.
    pub permission: Option<Permission>,
    /// LiveKit ignores an empty text: metadata can be replaced, not cleared.
    pub metadata: Option<String>,
    pub name: Option<String>,
}

/// How a data message is delivered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataOptions {
    /// Delivered in order with retransmission (default); `lossy` trades
    /// that for speed.
    pub lossy: bool,
    /// Lets receivers tell kinds of messages apart.
    pub topic: String,
    /// Identities to deliver to; empty means everyone in the room.
    pub destination_identities: Vec<String>,
}

/// What LiveKit posts to a webhook. `event` is one of `room_started`,
/// `room_finished`, `participant_joined`, `participant_left`,
/// `participant_connection_aborted`, `track_published`,
/// `track_unpublished` and the egress / ingress events.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct WebhookEvent {
    #[serde(default)]
    pub event: String,
    /// Unique per event; deliveries are retried.
    #[serde(default)]
    pub id: String,
    /// Unix seconds.
    #[serde(default, alias = "createdAt", deserialize_with = "lenient_i64")]
    pub created_at: i64,
    #[serde(default)]
    pub room: Option<Room>,
    #[serde(default)]
    pub participant: Option<Participant>,
    #[serde(default)]
    pub track: Option<Track>,
}

// ───────────────────────────── the client ─────────────────────────────

struct Deployment {
    http: reqwest::Client,
    api_url: String,
    public_url: String,
    api_key: String,
    api_secret: String,
}

/// The LiveKit deployment of this server, or the lack of one. Cheap to clone.
#[derive(Clone, Default)]
pub struct LiveKit {
    deployment: Option<Arc<Deployment>>,
}

#[derive(Deserialize)]
struct TwirpError {
    #[serde(default)]
    code: String,
    #[serde(default)]
    msg: String,
}

/// How a room-service call went wrong.
enum CallError {
    /// LiveKit knows no such room or participant.
    NotFound,
    Failed(AppError),
}

fn unavailable(reason: &'static str, message: String, cause: String) -> AppError {
    AppError {
        code: ErrorCode::Unavailable,
        reason: Cow::Borrowed(reason),
        message: Cow::Owned(message),
        source: Some(anyhow::anyhow!(cause)),
    }
}

fn not_configured() -> AppError {
    AppError::unavailable(NOT_CONFIGURED, "voice and video are not configured on this server")
}

fn webhook_unauthenticated(why: &'static str) -> AppError {
    AppError::unauthenticated(WEBHOOK_UNAUTHENTICATED, why)
}

impl From<CallError> for AppError {
    fn from(error: CallError) -> Self {
        match error {
            CallError::NotFound => {
                unavailable(REJECTED, "LiveKit refused the request".to_owned(), "not_found".to_owned())
            }
            CallError::Failed(error) => error,
        }
    }
}

/// `Ok(None)` for "no such room or participant".
fn found<T>(result: Result<T, CallError>) -> AppResult<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(CallError::NotFound) => Ok(None),
        Err(CallError::Failed(error)) => Err(error),
    }
}

impl LiveKit {
    /// A deployment reached at `api_url` by the server and at `public_url`
    /// (`ws://` / `wss://`) by clients.
    pub fn new(api_url: &str, public_url: &str, api_key: &str, api_secret: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(
            api_url.starts_with("http://") || api_url.starts_with("https://"),
            "TOKULY_LIVEKIT_URL must be an http(s) URL"
        );
        anyhow::ensure!(
            !public_url.is_empty() && !api_key.is_empty() && !api_secret.is_empty(),
            "a LiveKit deployment needs a public URL, an API key and an API secret"
        );
        let http = reqwest::Client::builder()
            .user_agent("Tokuly/1.0")
            .timeout(API_TIMEOUT)
            .build()
            .map_err(|e| anyhow::anyhow!("LiveKit http client: {e}"))?;
        Ok(Self {
            deployment: Some(Arc::new(Deployment {
                http,
                api_url: api_url.trim_end_matches('/').to_owned(),
                public_url: public_url.to_owned(),
                api_key: api_key.to_owned(),
                api_secret: api_secret.to_owned(),
            })),
        })
    }

    /// No LiveKit: every method fails with [`NOT_CONFIGURED`].
    pub fn disabled() -> Self {
        Self { deployment: None }
    }

    /// From the `TOKULY_LIVEKIT_*` settings: all four make a deployment,
    /// none of them [`Self::disabled`]. Some but not all is a mistake.
    pub fn from_config(cfg: &Config) -> anyhow::Result<Self> {
        let settings = [&cfg.livekit_url, &cfg.livekit_public_url, &cfg.livekit_api_key, &cfg.livekit_api_secret];
        let set = settings.iter().filter(|value| !value.is_empty()).count();
        if set == 0 {
            return Ok(Self::disabled());
        }
        anyhow::ensure!(
            set == settings.len(),
            "LiveKit needs all of TOKULY_LIVEKIT_URL, TOKULY_LIVEKIT_PUBLIC_URL, TOKULY_LIVEKIT_API_KEY and TOKULY_LIVEKIT_API_SECRET (or none, to run without voice and video)"
        );
        Self::new(&cfg.livekit_url, &cfg.livekit_public_url, &cfg.livekit_api_key, &cfg.livekit_api_secret)
    }

    /// Whether this server has a LiveKit deployment.
    pub fn is_configured(&self) -> bool {
        self.deployment.is_some()
    }

    fn deployment(&self) -> AppResult<&Deployment> {
        self.deployment.as_deref().ok_or_else(not_configured)
    }

    /// The URL clients connect to.
    pub fn public_url(&self) -> AppResult<&str> {
        Ok(&self.deployment()?.public_url)
    }

    // ───────────────────────────── tokens ─────────────────────────────

    fn sign(&self, claims: Claims) -> AppResult<String> {
        let deployment = self.deployment()?;
        let key = EncodingKey::from_secret(deployment.api_secret.as_bytes())
            .map_err(|e| AppError::internal(anyhow::anyhow!("LiveKit signing key: {e}")))?;
        jsonwebtoken::encode(&Header::new(Algorithm::HS256), &claims, &key)
            .map_err(|e| AppError::internal(anyhow::anyhow!("LiveKit token: {e}")))
    }

    fn claims(&self, ttl: Duration) -> AppResult<Claims> {
        let now = jsonwebtoken::get_current_timestamp();
        Ok(Claims {
            iss: self.deployment()?.api_key.clone(),
            nbf: now.saturating_sub(CLOCK_SLACK_SECONDS),
            exp: now + ttl.as_secs().max(1),
            ..Default::default()
        })
    }

    /// Mints the access token a client connects with.
    pub fn token(&self, options: &TokenOptions) -> AppResult<String> {
        let claims = Claims {
            sub: options.identity.clone(),
            name: options.name.clone(),
            metadata: options.metadata.clone(),
            video: options.grant(),
            ..self.claims(options.ttl)?
        };
        self.sign(claims)
    }

    /// A token for one of the server's own API calls.
    fn api_token(&self, video: VideoGrant) -> AppResult<String> {
        self.sign(Claims { video, ..self.claims(API_TOKEN_TTL)? })
    }

    /// Checks that `token` was signed by this deployment's key pair and is
    /// valid now, and returns its claims. For webhooks and for tests.
    pub fn decode_token(&self, token: &str) -> AppResult<Claims> {
        let deployment = self.deployment()?;
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&[deployment.api_key.as_str()]);
        validation.set_required_spec_claims(&["exp", "iss"]);
        validation.validate_nbf = true;
        validation.leeway = CLOCK_SLACK_SECONDS;
        let key = DecodingKey::from_secret(deployment.api_secret.as_bytes())
            .map_err(|e| AppError::internal(anyhow::anyhow!("LiveKit signing key: {e}")))?;
        jsonwebtoken::decode::<Claims>(token, &key, &validation)
            .map(|data| data.claims)
            .map_err(|_| AppError::unauthenticated("LIVEKIT_TOKEN_INVALID", "not a valid LiveKit token of this server"))
    }

    // ───────────────────────────── webhooks ─────────────────────────────

    /// Verifies a webhook request the way LiveKit specifies and returns its
    /// event: `authorization` is the `Authorization` header, a token signed
    /// with this deployment's key pair whose `sha256` claim is the base64
    /// SHA-256 of the raw request `body`.
    ///
    /// Fails with `unauthenticated` / [`WEBHOOK_UNAUTHENTICATED`] when the
    /// request is not LiveKit's, and `invalid_argument` /
    /// [`WEBHOOK_INVALID`] when the signed body is not an event.
    pub fn verify_webhook(&self, authorization: &str, body: &[u8]) -> AppResult<WebhookEvent> {
        self.deployment()?;
        let token = authorization.trim();
        let token = token.strip_prefix("Bearer ").unwrap_or(token);
        if token.is_empty() {
            return Err(webhook_unauthenticated("the webhook carries no signature"));
        }
        let claims =
            self.decode_token(token).map_err(|_| webhook_unauthenticated("the webhook signature is not valid"))?;
        let digest = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(body));
        if !bool::from(claims.sha256.as_bytes().ct_eq(digest.as_bytes())) {
            return Err(webhook_unauthenticated("the webhook body does not match its signature"));
        }
        serde_json::from_slice(body)
            .map_err(|e| AppError::invalid(WEBHOOK_INVALID, format!("not a webhook event: {e}")))
    }

    /// The `Authorization` header LiveKit would send with `body`. For tests
    /// and tools that replay webhooks.
    pub fn sign_webhook(&self, body: &[u8]) -> AppResult<String> {
        let sha256 = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(body));
        self.sign(Claims { sha256, ..self.claims(Duration::from_secs(300))? })
    }

    // ───────────────────────────── room service ─────────────────────────────

    async fn call<T: DeserializeOwned>(
        &self,
        method: &str,
        grant: VideoGrant,
        body: serde_json::Value,
    ) -> Result<T, CallError> {
        let deployment = self.deployment().map_err(CallError::Failed)?;
        let token = self.api_token(grant).map_err(CallError::Failed)?;
        let unreachable =
            |cause: String| CallError::Failed(unavailable(UNAVAILABLE, format!("LiveKit {method} failed"), cause));
        let response = deployment
            .http
            .post(format!("{}/twirp/livekit.RoomService/{method}", deployment.api_url))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(|e| unreachable(e.to_string()))?;
        let status = response.status();
        let bytes = response.bytes().await.map_err(|e| unreachable(e.to_string()))?;
        if status.is_success() {
            return serde_json::from_slice(&bytes).map_err(|e| unreachable(format!("undecodable answer: {e}")));
        }
        // A Twirp error says what went wrong in `code`. A token LiveKit does
        // not accept is answered before Twirp is reached: a plain `401`.
        let detail = match serde_json::from_slice::<TwirpError>(&bytes) {
            Ok(error) if !error.code.is_empty() => {
                if error.code == "not_found" {
                    return Err(CallError::NotFound);
                }
                format!("{}: {}", error.code, error.msg)
            }
            _ if status == reqwest::StatusCode::NOT_FOUND => return Err(CallError::NotFound),
            _ => String::from_utf8_lossy(&bytes).trim().chars().take(200).collect(),
        };
        let cause = format!("HTTP {}: {detail}", status.as_u16());
        Err(if status.is_client_error() {
            if matches!(status.as_u16(), 401 | 403) {
                tracing::error!(method, %cause, "LiveKit rejected the server's token; check TOKULY_LIVEKIT_API_KEY / _SECRET");
            }
            CallError::Failed(unavailable(REJECTED, format!("LiveKit refused {method}"), cause))
        } else {
            unreachable(cause)
        })
    }

    /// Creates a room, or returns the room of that name if it exists (its
    /// settings are then left as they are).
    pub async fn create_room(&self, options: &RoomOptions) -> AppResult<Room> {
        let grant = VideoGrant { room_create: true, ..Default::default() };
        let mut body = serde_json::json!({
            "name": options.name,
            "empty_timeout": options.empty_timeout_seconds,
            "departure_timeout": options.departure_timeout_seconds,
            "max_participants": options.max_participants,
        });
        if !options.metadata.is_empty() {
            body["metadata"] = serde_json::Value::String(options.metadata.clone());
        }
        Ok(self.call("CreateRoom", grant, body).await?)
    }

    /// Closes a room, disconnecting everyone in it. `false` when there was
    /// no such room.
    pub async fn delete_room(&self, room: &str) -> AppResult<bool> {
        let grant = VideoGrant { room_create: true, ..Default::default() };
        let deleted = self.call::<serde_json::Value>("DeleteRoom", grant, serde_json::json!({ "room": room })).await;
        Ok(found(deleted)?.is_some())
    }

    /// The rooms that exist among `names`, or every room when `names` is
    /// empty.
    pub async fn list_rooms(&self, names: &[String]) -> AppResult<Vec<Room>> {
        #[derive(Deserialize)]
        struct Rooms {
            #[serde(default, deserialize_with = "null_as_default")]
            rooms: Vec<Room>,
        }
        let grant = VideoGrant { room_list: true, ..Default::default() };
        let listed: Rooms = self.call("ListRooms", grant, serde_json::json!({ "names": names })).await?;
        Ok(listed.rooms)
    }

    /// Everyone connected to a room. Empty when nobody is, which is also
    /// what a room that does not exist looks like.
    pub async fn list_participants(&self, room: &str) -> AppResult<Vec<Participant>> {
        #[derive(Deserialize)]
        struct Participants {
            #[serde(default, deserialize_with = "null_as_default")]
            participants: Vec<Participant>,
        }
        let grant = VideoGrant { room: room.to_owned(), room_admin: true, ..Default::default() };
        let listed = self.call::<Participants>("ListParticipants", grant, serde_json::json!({ "room": room })).await;
        Ok(found(listed)?.map(|listed| listed.participants).unwrap_or_default())
    }

    /// One participant; `None` when they are not connected to the room.
    pub async fn get_participant(&self, room: &str, identity: &str) -> AppResult<Option<Participant>> {
        let grant = VideoGrant { room: room.to_owned(), room_admin: true, ..Default::default() };
        found(self.call("GetParticipant", grant, serde_json::json!({ "room": room, "identity": identity })).await)
    }

    /// Disconnects a participant. `false` when they were not connected.
    ///
    /// Their token stays valid until it expires: a participant who must
    /// stay out has to be refused on the way in as well (a webhook, or a
    /// comparison of `list_participants` with who is allowed).
    pub async fn remove_participant(&self, room: &str, identity: &str) -> AppResult<bool> {
        let grant = VideoGrant { room: room.to_owned(), room_admin: true, ..Default::default() };
        let body = serde_json::json!({ "room": room, "identity": identity });
        Ok(found(self.call::<serde_json::Value>("RemoveParticipant", grant, body).await)?.is_some())
    }

    /// Changes a connected participant's permission, metadata or name; the
    /// participant and everyone else in the room learn of it at once.
    /// `None` when they are not connected.
    pub async fn update_participant(
        &self,
        room: &str,
        identity: &str,
        update: &ParticipantUpdate,
    ) -> AppResult<Option<Participant>> {
        let grant = VideoGrant { room: room.to_owned(), room_admin: true, ..Default::default() };
        let mut body = serde_json::json!({ "room": room, "identity": identity });
        if let Some(permission) = &update.permission {
            body["permission"] = serde_json::to_value(permission)
                .map_err(|e| AppError::internal(anyhow::anyhow!("LiveKit permission: {e}")))?;
        }
        if let Some(metadata) = &update.metadata {
            body["metadata"] = serde_json::Value::String(metadata.clone());
        }
        if let Some(name) = &update.name {
            body["name"] = serde_json::Value::String(name.clone());
        }
        found(self.call("UpdateParticipant", grant, body).await)
    }

    /// Mutes (or unmutes) one published track from the server. `None` when
    /// the participant publishes no such track.
    ///
    /// LiveKit refuses to unmute remotely unless the server is started with
    /// `room.enable_remote_unmute`. Muting a participant who is not
    /// connected fails with [`UNAVAILABLE`]: nothing answers for them.
    pub async fn mute_track(
        &self,
        room: &str,
        identity: &str,
        track_sid: &str,
        muted: bool,
    ) -> AppResult<Option<Track>> {
        #[derive(Deserialize)]
        struct Muted {
            #[serde(default)]
            track: Option<Track>,
        }
        let grant = VideoGrant { room: room.to_owned(), room_admin: true, ..Default::default() };
        let body = serde_json::json!({ "room": room, "identity": identity, "track_sid": track_sid, "muted": muted });
        Ok(found(self.call::<Muted>("MutePublishedTrack", grant, body).await)?.and_then(|muted| muted.track))
    }

    /// Sends a data message to the participants of a room (clients receive
    /// it as a data packet from the server). Sending to a room nobody is
    /// connected to fails with [`UNAVAILABLE`].
    pub async fn send_data(&self, room: &str, data: &[u8], options: &DataOptions) -> AppResult<()> {
        let grant = VideoGrant { room: room.to_owned(), room_admin: true, ..Default::default() };
        let mut body = serde_json::json!({
            "room": room,
            "data": base64::engine::general_purpose::STANDARD.encode(data),
            "kind": if options.lossy { "LOSSY" } else { "RELIABLE" },
            "destination_identities": options.destination_identities,
        });
        if !options.topic.is_empty() {
            body["topic"] = serde_json::Value::String(options.topic.clone());
        }
        self.call::<serde_json::Value>("SendData", grant, body).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "a-secret-of-at-least-thirty-two-characters";

    fn livekit() -> LiveKit {
        // Nothing listens there: the tests below never leave the process.
        LiveKit::new("http://127.0.0.1:9/", "ws://voice.example", "devkey", SECRET).unwrap()
    }

    fn config(url: &str, public_url: &str, key: &str, secret: &str) -> Config {
        use clap::Parser;
        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            cfg: Config,
        }
        let mut cfg = Cli::parse_from(["test", "--secret", "0123456789abcdef0123456789abcdef"]).cfg;
        cfg.livekit_url = url.to_owned();
        cfg.livekit_public_url = public_url.to_owned();
        cfg.livekit_api_key = key.to_owned();
        cfg.livekit_api_secret = secret.to_owned();
        cfg
    }

    /// The claims exactly as LiveKit reads them.
    fn raw_claims(token: &str) -> serde_json::Value {
        jsonwebtoken::dangerous::insecure_decode::<serde_json::Value>(token).unwrap().claims
    }

    #[test]
    fn configuration_is_all_or_nothing() {
        assert!(!LiveKit::from_config(&config("", "", "", "")).unwrap().is_configured());
        let configured =
            LiveKit::from_config(&config("http://livekit:7880/", "wss://voice.example", "key", SECRET)).unwrap();
        assert!(configured.is_configured());
        assert_eq!(configured.public_url().unwrap(), "wss://voice.example");
        assert_eq!(configured.deployment().unwrap().api_url, "http://livekit:7880", "the trailing slash is dropped");
        assert!(LiveKit::from_config(&config("http://livekit:7880", "", "key", SECRET)).is_err(), "partial");
        assert!(LiveKit::from_config(&config("livekit:7880", "ws://x", "key", SECRET)).is_err(), "not a URL");
    }

    #[test]
    fn a_join_token_carries_identity_and_the_room_grant() {
        let livekit = livekit();
        let options = TokenOptions::join("room-1", "user-1").name("たろう").metadata(r#"{"role":"host"}"#);
        let token = livekit.token(&options).unwrap();
        assert_eq!(jsonwebtoken::decode_header(&token).unwrap().alg, Algorithm::HS256);

        let claims = livekit.decode_token(&token).unwrap();
        assert_eq!(claims.iss, "devkey");
        assert_eq!(claims.sub, "user-1");
        assert_eq!(claims.name, "たろう");
        assert_eq!(claims.metadata, r#"{"role":"host"}"#);
        assert!(claims.sha256.is_empty());
        assert_eq!(
            claims.video,
            VideoGrant {
                room: "room-1".into(),
                room_join: true,
                can_publish: Some(true),
                can_subscribe: Some(true),
                can_publish_data: Some(true),
                ..Default::default()
            }
        );
        let now = jsonwebtoken::get_current_timestamp();
        let lifetime = claims.exp - now;
        assert!((595..=600).contains(&lifetime), "expires in ten minutes, got {lifetime}s");
        assert!(claims.nbf <= now && now - claims.nbf <= 15, "valid from a moment ago");
    }

    #[test]
    fn a_narrowed_token_says_what_is_refused() {
        let livekit = livekit();
        let listener = TokenOptions::join("room-1", "user-2")
            .publish(false)
            .publish_data(false)
            .sources(&[TrackSource::Microphone])
            .ttl(Duration::from_secs(120));
        let claims = raw_claims(&livekit.token(&listener).unwrap());
        // LiveKit grants what a token leaves out, so `false` is spelled out.
        assert_eq!(
            claims["video"],
            serde_json::json!({
                "room": "room-1",
                "roomJoin": true,
                "canPublish": false,
                "canSubscribe": true,
                "canPublishData": false,
                "canPublishSources": ["microphone"],
            })
        );
        assert!(claims.get("name").is_none() && claims.get("metadata").is_none() && claims.get("sha256").is_none());
        let lifetime = claims["exp"].as_u64().unwrap() - jsonwebtoken::get_current_timestamp();
        assert!((115..=120).contains(&lifetime));

        let moderator = TokenOptions::join("room-1", "mod").subscribe(false).admin(true).hidden(true);
        let claims = raw_claims(&livekit.token(&moderator).unwrap());
        assert_eq!(claims["video"]["roomAdmin"], true);
        assert_eq!(claims["video"]["hidden"], true);
        assert_eq!(claims["video"]["canSubscribe"], false);
        assert_eq!(
            Permission::of(&listener),
            Permission {
                can_subscribe: true,
                can_publish: false,
                can_publish_data: false,
                can_publish_sources: vec![TrackSource::Microphone],
                hidden: false,
                can_update_metadata: false,
            }
        );
    }

    #[test]
    fn api_tokens_have_no_participant() {
        let livekit = livekit();
        let token = livekit.api_token(VideoGrant { room: "r".into(), room_admin: true, ..Default::default() }).unwrap();
        let claims = raw_claims(&token);
        assert_eq!(claims["video"], serde_json::json!({ "room": "r", "roomAdmin": true }));
        assert!(claims.get("sub").is_none(), "no participant identity");
        let lifetime = claims["exp"].as_u64().unwrap() - jsonwebtoken::get_current_timestamp();
        assert!(lifetime <= 60);
    }

    #[test]
    fn tokens_of_another_key_pair_are_refused() {
        let livekit = livekit();
        let token = livekit.token(&TokenOptions::join("room-1", "user-1")).unwrap();
        let other_secret =
            LiveKit::new("http://127.0.0.1:9", "ws://x", "devkey", "another-secret-of-thirty-two-characters").unwrap();
        let other_key = LiveKit::new("http://127.0.0.1:9", "ws://x", "otherkey", SECRET).unwrap();
        for stranger in [other_secret, other_key] {
            let error = stranger.decode_token(&token).unwrap_err();
            assert_eq!((error.code, error.reason.as_ref()), (ErrorCode::Unauthenticated, "LIVEKIT_TOKEN_INVALID"));
        }
        let expired = livekit.sign(Claims { iss: "devkey".into(), nbf: 1, exp: 2, ..Default::default() }).unwrap();
        assert!(livekit.decode_token(&expired).is_err());
        assert!(livekit.decode_token("not-a-token").is_err());
    }

    #[tokio::test]
    async fn everything_is_unavailable_without_configuration() {
        let livekit = LiveKit::disabled();
        assert!(!livekit.is_configured() && !LiveKit::default().is_configured());
        let refused = |error: AppError| {
            assert_eq!((error.code, error.reason.as_ref()), (ErrorCode::Unavailable, NOT_CONFIGURED), "{error}");
        };
        refused(livekit.public_url().unwrap_err());
        refused(livekit.token(&TokenOptions::join("r", "u")).unwrap_err());
        refused(livekit.decode_token("x").unwrap_err());
        refused(livekit.verify_webhook("x", b"{}").unwrap_err());
        refused(livekit.sign_webhook(b"{}").unwrap_err());
        refused(livekit.create_room(&RoomOptions::named("r")).await.unwrap_err());
        refused(livekit.delete_room("r").await.unwrap_err());
        refused(livekit.list_rooms(&[]).await.unwrap_err());
        refused(livekit.list_participants("r").await.unwrap_err());
        refused(livekit.get_participant("r", "u").await.unwrap_err());
        refused(livekit.remove_participant("r", "u").await.unwrap_err());
        refused(livekit.update_participant("r", "u", &ParticipantUpdate::default()).await.unwrap_err());
        refused(livekit.mute_track("r", "u", "TR_x", true).await.unwrap_err());
        refused(livekit.send_data("r", b"hi", &DataOptions::default()).await.unwrap_err());
    }

    #[tokio::test]
    async fn an_unreachable_server_is_unavailable() {
        let error = livekit().list_rooms(&[]).await.unwrap_err();
        assert_eq!((error.code, error.reason.as_ref()), (ErrorCode::Unavailable, UNAVAILABLE), "{error}");
    }

    #[test]
    fn webhooks_are_verified_against_signature_and_body() {
        let livekit = livekit();
        // What LiveKit posts: protobuf JSON with camelCase names and 64-bit numbers as text.
        let body = br#"{"event":"participant_joined","room":{"sid":"RM_1","name":"room-1","numParticipants":1,"creationTime":"1795281565"},"participant":{"sid":"PA_1","identity":"user-1","state":"ACTIVE","joinedAt":"1795281570","permission":{"canSubscribe":true,"canPublish":true,"canPublishSources":["MICROPHONE"]},"isPublisher":true},"id":"EV_1","createdAt":"1795281571"}"#;
        let authorization = livekit.sign_webhook(body).unwrap();
        assert_eq!(raw_claims(&authorization)["sha256"].as_str().unwrap().len(), 44, "base64 of 32 bytes");

        let event = livekit.verify_webhook(&authorization, body).unwrap();
        assert_eq!(
            (event.event.as_str(), event.id.as_str(), event.created_at),
            ("participant_joined", "EV_1", 1795281571)
        );
        let room = event.room.unwrap();
        assert_eq!((room.name.as_str(), room.num_participants, room.creation_time), ("room-1", 1, 1795281565));
        let participant = event.participant.unwrap();
        assert_eq!(participant.identity, "user-1");
        assert_eq!(participant.state, ParticipantState::Active);
        assert_eq!(participant.joined_at, 1795281570);
        assert!(participant.is_publisher);
        let permission = participant.permission.unwrap();
        assert!(permission.can_subscribe && permission.can_publish && !permission.can_publish_data);
        assert_eq!(permission.can_publish_sources, vec![TrackSource::Microphone]);
        assert!(event.track.is_none());
        // Some senders prefix the scheme.
        assert!(livekit.verify_webhook(&format!("Bearer {authorization}"), body).is_ok());

        let rejected = |authorization: &str, body: &[u8]| {
            let error = livekit.verify_webhook(authorization, body).unwrap_err();
            assert_eq!(
                (error.code, error.reason.as_ref()),
                (ErrorCode::Unauthenticated, WEBHOOK_UNAUTHENTICATED),
                "{error}"
            );
        };
        rejected(&authorization, br#"{"event":"room_finished"}"#);
        rejected("", body);
        rejected("garbage", body);
        // A join token is signed by the right key but says nothing about the body.
        rejected(&livekit.token(&TokenOptions::join("room-1", "user-1")).unwrap(), body);
        let stranger =
            LiveKit::new("http://127.0.0.1:9", "ws://x", "devkey", "another-secret-of-thirty-two-characters").unwrap();
        rejected(&stranger.sign_webhook(body).unwrap(), body);

        let not_an_event = b"[1, 2]";
        let error = livekit.verify_webhook(&livekit.sign_webhook(not_an_event).unwrap(), not_an_event).unwrap_err();
        assert_eq!((error.code, error.reason.as_ref()), (ErrorCode::InvalidArgument, WEBHOOK_INVALID));
    }

    #[test]
    fn room_service_answers_are_understood() {
        // As the room service answers: protobuf field names, unset messages as null.
        let participant: Participant = serde_json::from_str(
            r#"{"sid":"PA_XkPYgHcenzDF", "identity":"user-a", "state":"JOINED", "tracks":[{"sid":"TR_1", "type":"AUDIO", "name":"mic", "muted":true, "source":"MICROPHONE"}, {"sid":"TR_2", "type":"SOMETHING_NEW", "source":"HOLOGRAM"}], "metadata":"{\"role\":\"listener\"}", "joined_at":"1795281565", "joined_at_ms":"1795281565635", "name":"A", "version":1, "permission":{"can_subscribe":true, "can_publish":false, "can_publish_data":false, "can_publish_sources":[], "hidden":false, "recorder":false, "can_update_metadata":false, "agent":false}, "region":"", "is_publisher":false, "kind":"STANDARD", "attributes":{}, "disconnect_reason":"UNKNOWN_REASON"}"#,
        )
        .unwrap();
        assert_eq!(participant.identity, "user-a");
        assert_eq!(participant.state, ParticipantState::Joined);
        assert_eq!(participant.metadata, r#"{"role":"listener"}"#);
        assert_eq!(participant.joined_at, 1795281565);
        assert_eq!(participant.permission, Some(Permission { can_subscribe: true, ..Default::default() }));
        assert_eq!(
            participant.tracks[0],
            Track {
                sid: "TR_1".into(),
                kind: TrackKind::Audio,
                name: "mic".into(),
                muted: true,
                source: TrackSource::Microphone
            }
        );
        assert_eq!(
            (participant.tracks[1].kind, participant.tracks[1].source),
            (TrackKind::Unknown, TrackSource::Unknown)
        );

        let room: Room = serde_json::from_str(
            r#"{"sid":"RM_Ck6UWaJhAH4q", "name":"room-1", "empty_timeout":60, "departure_timeout":5, "max_participants":12, "creation_time":"1795281536", "metadata":null, "num_participants":0, "num_publishers":0, "active_recording":false, "version":null}"#,
        )
        .unwrap();
        assert_eq!((room.empty_timeout, room.departure_timeout, room.max_participants), (60, 5, 12));
        assert_eq!((room.creation_time, room.metadata.as_str()), (1795281536, ""));

        let sparse: Participant =
            serde_json::from_str(r#"{"identity":"user-b","permission":null,"tracks":null}"#).unwrap();
        assert_eq!(sparse.state, ParticipantState::Joining);
        assert!(sparse.permission.is_none() && sparse.tracks.is_empty());
    }

    #[test]
    fn permissions_are_sent_under_protobuf_names() {
        let permission = Permission {
            can_subscribe: true,
            can_publish: true,
            can_publish_data: false,
            can_publish_sources: vec![TrackSource::Microphone, TrackSource::ScreenShareAudio],
            hidden: false,
            can_update_metadata: false,
        };
        assert_eq!(
            serde_json::to_value(&permission).unwrap(),
            serde_json::json!({
                "can_subscribe": true,
                "can_publish": true,
                "can_publish_data": false,
                "can_publish_sources": ["MICROPHONE", "SCREEN_SHARE_AUDIO"],
                "hidden": false,
                "can_update_metadata": false,
            })
        );
    }
}
