//! Payloads of the events this domain emits and consumes
//! (`docs/events/dev.md`).
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// `dev.token.created` / `dev.token.revoked`: a personal access token or a
/// bot token. The actor is the person who created or revoked it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenEvent {
    pub token_id: Uuid,
    /// The account the token acts as (the bot, for a bot token).
    pub user_id: Uuid,
    /// `pat` or `bot`.
    pub kind: String,
    /// Why it was revoked: `owner`, `password_changed`, `banned`,
    /// `bot_deleted`. Empty on creation.
    #[serde(default)]
    pub reason: String,
}

/// `dev.app.created` / `.updated` / `.deleted` / `.disabled`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppEvent {
    pub app_id: Uuid,
    pub owner_id: Uuid,
}

/// `dev.authorization.granted` / `.revoked`: a user's permission for an
/// application.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationEvent {
    pub app_id: Uuid,
    pub user_id: Uuid,
    pub scopes: Vec<String>,
    /// On revocation: `user` or `banned`. Empty when granted.
    #[serde(default)]
    pub reason: String,
}

/// `dev.bot.created` / `.updated` / `.deleted`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotEvent {
    pub bot_id: Uuid,
    pub owner_id: Uuid,
}

/// `dev.bot.transferred`: a bot was handed over (`from_id` to `to_id`).
/// Actor: the receiver, who accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotTransferEvent {
    pub bot_id: Uuid,
    pub from_id: Uuid,
    pub to_id: Uuid,
}

/// `dev.webhook.disabled`: deliveries kept failing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookDisabled {
    pub webhook_id: Uuid,
    pub account_id: Uuid,
}

/// `identity.user.*` events carry just the user.
#[derive(Debug, Clone, Deserialize)]
pub struct UserRef {
    pub user_id: Uuid,
}

/// `admin.content.removed`.
#[derive(Debug, Clone, Deserialize)]
pub struct ContentRemoved {
    pub subject_type: i32,
    pub subject_id: Uuid,
    #[serde(default)]
    pub reason: String,
}

/// The job that makes one attempt at a webhook delivery (`dev.webhook.deliver`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliverJob {
    pub delivery_id: Uuid,
}
