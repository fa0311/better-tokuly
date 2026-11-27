//! Webhook subscriptions (`dev_webhooks`), the catalogue of event types
//! and `WebhookService`.
//!
//! A webhook belongs to an account — a person or a bot — and receives that
//! account's events. `fanout.rs` turns domain events into deliveries and
//! `delivery.rs` sends them.
use std::time::Duration;

use chrono::Utc;
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::{auth::AuthContext, prelude::*};
use tokuly_proto::{common::LocalizedText, dev::*};

use crate::{
    Dev, clean_text,
    delivery::{self, DeliveryRow},
    lock_limit,
    secrets::{self, SecretKind},
};

const MAX_PAGE_SIZE: i64 = 100;
const MAX_URL_LEN: usize = 2000;

/// Where the events of a type come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A domain event. It is delivered to the accounts named by these
    /// fields of its payload (each a user ID or a list of them), with the
    /// payload as `data`.
    Topic { topic: &'static str, accounts: &'static [&'static str] },
    /// A notification addressed to the account whose kind is this, or
    /// starts with this and a dot; empty matches every notification.
    Notification { kind: &'static str },
}

/// A kind of event a webhook can subscribe to.
#[derive(Debug, Clone, Copy)]
pub struct EventType {
    pub name: &'static str,
    /// What a token needs in order to read deliveries of the type: the
    /// scopes that reading the same thing through the API would take.
    pub scopes: &'static [&'static str],
    pub ja: &'static str,
    pub en: &'static str,
    pub source: Source,
}

/// Sent by `PingWebhook`; not something to subscribe to.
pub const PING: &str = "ping";

/// The catalogue. Every entry is about the receiving account itself: what
/// was sent to it, done to it, or done by it.
pub const EVENT_TYPES: &[EventType] = &[
    EventType {
        name: "notification.created",
        // Any notification: message previews and place alerts among them.
        scopes: &["notify:read", "chat:read", "geo:read"],
        ja: "通知が届いた（すべての種類）",
        en: "A notification arrived (every kind)",
        source: Source::Notification { kind: "" },
    },
    EventType {
        name: "friend.request_received",
        scopes: &["social:read"],
        ja: "友達リクエストが届いた",
        en: "Someone sent you a friend request",
        source: Source::Topic { topic: "social.friend.requested", accounts: &["to_user_id"] },
    },
    EventType {
        name: "friend.added",
        scopes: &["social:read"],
        ja: "友達になった",
        en: "You became friends with someone",
        source: Source::Topic { topic: "social.friend.accepted", accounts: &["user_id", "friend_id"] },
    },
    EventType {
        name: "friend.removed",
        scopes: &["social:read"],
        ja: "友達を削除した",
        en: "You removed a friend",
        source: Source::Topic { topic: "social.friend.removed", accounts: &["user_id"] },
    },
    EventType {
        name: "chat.message_received",
        scopes: &["chat:read"],
        ja: "メッセージが届いた",
        en: "You received a message",
        source: Source::Notification { kind: "chat.message" },
    },
    EventType {
        name: "chat.mentioned",
        scopes: &["chat:read"],
        ja: "チャットでメンションされた",
        en: "You were mentioned in a chat",
        source: Source::Notification { kind: "chat.mention" },
    },
    EventType {
        name: "chat.call_incoming",
        scopes: &["chat:read"],
        ja: "着信があった",
        en: "Someone is calling you",
        source: Source::Notification { kind: "chat.call" },
    },
    EventType {
        name: "place.alert",
        scopes: &["geo:read"],
        ja: "見守りアラート：友達が場所に到着・出発した",
        en: "A place alert fired: a friend arrived at or left a place",
        source: Source::Notification { kind: "geo.place_alert" },
    },
    EventType {
        name: "channel.video_published",
        scopes: &["video:read"],
        ja: "登録チャンネルが動画を公開した",
        en: "A channel you subscribe to published a video",
        source: Source::Notification { kind: "video.published" },
    },
    EventType {
        name: "channel.subscriber_added",
        scopes: &["video:read"],
        ja: "あなたのチャンネルが登録された",
        en: "Someone subscribed to your channel",
        source: Source::Topic { topic: "video.channel.subscribed", accounts: &["owner_id"] },
    },
    EventType {
        name: "note.created",
        scopes: &["notes:read"],
        ja: "ノートを投稿した",
        en: "You posted a note",
        source: Source::Topic { topic: "notes.note.created", accounts: &["author_id"] },
    },
    EventType {
        name: "note.mentioned",
        scopes: &["notes:read"],
        ja: "ノートでメンションされた",
        en: "You were mentioned in a note",
        source: Source::Notification { kind: "notes.mention" },
    },
    EventType {
        name: "note.replied",
        scopes: &["notes:read"],
        ja: "ノートに返信が届いた",
        en: "Someone replied to your note",
        source: Source::Notification { kind: "notes.reply" },
    },
    EventType {
        name: "comment.received",
        scopes: &["engagement:read"],
        ja: "あなたの投稿にコメントが付いた",
        en: "Someone commented on something of yours",
        source: Source::Topic { topic: "engagement.comment.published", accounts: &["subject_owner_id"] },
    },
    EventType {
        name: "reaction.received",
        scopes: &["engagement:read"],
        ja: "あなたの投稿にリアクションが付いた",
        en: "Someone reacted to something of yours",
        source: Source::Topic { topic: "engagement.reaction.changed", accounts: &["subject_owner_id"] },
    },
    EventType {
        name: "wallet.balance_changed",
        scopes: &["wallet:read"],
        ja: "コイン残高が変わった",
        en: "Your coin balance changed",
        source: Source::Topic { topic: "wallet.balance.changed", accounts: &["user_id"] },
    },
    EventType {
        name: "account.updated",
        scopes: &["identity:read"],
        ja: "プロフィールが更新された",
        en: "Your profile changed",
        source: Source::Topic { topic: "identity.user.updated", accounts: &["user_id"] },
    },
];

pub fn event_type(name: &str) -> Option<&'static EventType> {
    EVENT_TYPES.iter().find(|event_type| event_type.name == name)
}

/// Whether the caller may see deliveries of an event type: a session may;
/// a token only with every scope the type needs. A `ping` carries nothing.
pub(crate) fn may_read(auth: &AuthContext, event_type_name: &str) -> bool {
    match event_type(event_type_name) {
        Some(event_type) => event_type.scopes.iter().all(|scope| auth.allows(scope)),
        None => auth.is_session() || event_type_name == PING,
    }
}

/// The event types whose deliveries a token may read; `None` for a session (all).
fn readable_event_types(auth: &AuthContext) -> Option<Vec<String>> {
    if auth.is_session() {
        return None;
    }
    Some(
        EVENT_TYPES
            .iter()
            .map(|t| t.name)
            .chain([PING])
            .filter(|name| may_read(auth, name))
            .map(str::to_owned)
            .collect(),
    )
}

impl EventType {
    fn to_proto(self) -> WebhookEventType {
        WebhookEventType {
            r#type: self.name.to_owned(),
            description: LocalizedText { ja: self.ja.to_owned(), en: self.en.to_owned(), ..Default::default() }.into(),
            scope: self.scopes.join(" "),
            payload: match self.source {
                Source::Topic { topic, .. } => topic.to_owned(),
                Source::Notification { .. } => "notification".to_owned(),
            },
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct WebhookRow {
    pub id: Uuid,
    pub account_id: Uuid,
    pub url: String,
    pub description: String,
    pub event_types: Vec<String>,
    pub secret_sealed: Vec<u8>,
    pub secret_hint: String,
    pub previous_secret_sealed: Option<Vec<u8>>,
    pub previous_secret_expires_at: Option<Time>,
    pub status: String,
    pub disabled_reason: String,
    pub consecutive_failures: i32,
    pub last_success_at: Option<Time>,
    pub last_failure_at: Option<Time>,
    pub created_at: Time,
    pub updated_at: Time,
}

pub(crate) const WEBHOOK_COLUMNS: &str = "id, account_id, url, description, event_types, secret_sealed, secret_hint, previous_secret_sealed, \
     previous_secret_expires_at, status, disabled_reason, consecutive_failures, last_success_at, last_failure_at, created_at, updated_at";

/// Label under which signing secrets are encrypted at rest.
const SEAL_LABEL: &str = "dev/webhook-secret";

pub(crate) const SEALED: [tokuly_core::rekey::Sealed; 2] = [
    tokuly_core::rekey::Sealed::bytes("dev_webhooks", "secret_sealed", SEAL_LABEL),
    tokuly_core::rekey::Sealed::bytes("dev_webhooks", "previous_secret_sealed", SEAL_LABEL),
];

impl WebhookRow {
    pub(crate) fn is_active(&self) -> bool {
        self.status == "active"
    }

    /// The secrets deliveries are signed with right now: the current one
    /// and, shortly after a rotation, the one it replaced.
    pub(crate) fn signing_secrets(&self, dev: &Dev) -> Vec<String> {
        let open =
            |sealed: &[u8]| dev.core.keys.open(SEAL_LABEL, sealed).and_then(|bytes| String::from_utf8(bytes).ok());
        let mut secrets: Vec<String> = open(&self.secret_sealed).into_iter().collect();
        if let (Some(previous), true) =
            (&self.previous_secret_sealed, self.previous_secret_expires_at.is_some_and(|at| at > Utc::now()))
        {
            secrets.extend(open(previous));
        }
        secrets
    }

    pub(crate) fn to_proto(&self) -> Webhook {
        let status = match self.status.as_str() {
            "active" => WebhookStatus::WEBHOOK_STATUS_ACTIVE,
            "paused" => WebhookStatus::WEBHOOK_STATUS_PAUSED,
            _ => WebhookStatus::WEBHOOK_STATUS_DISABLED,
        };
        Webhook {
            id: self.id.to_string(),
            account_id: self.account_id.to_string(),
            url: self.url.clone(),
            description: self.description.clone(),
            event_types: self.event_types.clone(),
            status: status.into(),
            disabled_reason: self.disabled_reason.clone(),
            secret_hint: self.secret_hint.clone(),
            created_at: ts(self.created_at),
            updated_at: ts(self.updated_at),
            last_success_at: opt_ts(self.last_success_at),
            last_failure_at: opt_ts(self.last_failure_at),
            consecutive_failures: self.consecutive_failures,
            previous_secret_expires_at: opt_ts(self.previous_secret_expires_at.filter(|at| *at > Utc::now())),
            ..Default::default()
        }
    }
}

/// The validated, editable part of a webhook.
struct WebhookFields {
    url: String,
    description: String,
    event_types: Vec<String>,
}

impl Dev {
    fn check_webhook(&self, url: &str, description: &str, event_types: &[String]) -> AppResult<WebhookFields> {
        let url = url.trim();
        ensure!(url.len() <= MAX_URL_LEN, invalid, "WEBHOOK_URL_INVALID", "the URL is too long");
        delivery::check_url(url, self.cfg.webhook_allow_private)
            .map_err(|why| AppError::invalid("WEBHOOK_URL_INVALID", format!("the URL {why}")))?;
        let description = clean_text(description, 200, "DESCRIPTION_TOO_LONG", "the description")?;
        ensure!(!event_types.is_empty(), invalid, "EVENT_TYPES_REQUIRED", "choose at least one event type");
        let mut names = Vec::new();
        for name in event_types {
            ensure!(event_type(name).is_some(), invalid, "EVENT_TYPE_UNKNOWN", "unknown event type {name:?}");
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
        Ok(WebhookFields { url: url.to_owned(), description, event_types: names })
    }

    /// The account a webhook request is about: the caller, or — signed in
    /// with a session — one of the caller's bots.
    async fn webhook_account(&self, ctx: &RequestContext, bot_id: &str) -> AppResult<Uuid> {
        if bot_id.is_empty() {
            return ctx.user_id();
        }
        let owner = ctx.require_session()?.user_id;
        Ok(self.owned_bot(parse_id(bot_id)?, owner).await?.user_id)
    }

    /// A webhook the caller may manage: their own, or one of a bot they own.
    pub(crate) async fn owned_webhook(&self, ctx: &RequestContext, webhook_id: &str) -> AppResult<WebhookRow> {
        let auth = ctx.auth()?;
        let row: Option<WebhookRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {WEBHOOK_COLUMNS} FROM dev_webhooks w
             WHERE w.id = $1 AND (w.account_id = $2 OR ($3 AND EXISTS(SELECT 1 FROM dev_bots b WHERE b.user_id = w.account_id AND b.owner_id = $2)))"
        )))
        .bind(parse_id(webhook_id)?)
        .bind(auth.user_id)
        .bind(auth.is_session())
        .fetch_optional(&self.core.db)
        .await?;
        row.or_not_found("WEBHOOK_NOT_FOUND", "webhook not found")
    }

    async fn webhook(&self, id: Uuid) -> AppResult<WebhookRow> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {WEBHOOK_COLUMNS} FROM dev_webhooks WHERE id = $1")))
            .bind(id)
            .fetch_one(&self.core.db)
            .await?)
    }

    fn seal_secret(&self, secret: &str) -> Vec<u8> {
        self.core.keys.seal(SEAL_LABEL, secret.as_bytes())
    }
}

impl WebhookService for Dev {
    async fn list_event_types(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, ListEventTypesRequest>,
    ) -> ServiceResult<ListEventTypesResponse> {
        handle(async {
            ok(ListEventTypesResponse {
                event_types: EVENT_TYPES.iter().map(|t| t.to_proto()).collect(),
                ..Default::default()
            })
        })
        .await
    }

    async fn create_webhook(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateWebhookRequest>,
    ) -> ServiceResult<CreateWebhookResponse> {
        handle(async {
            // Where a person's events go is decided by the person, at a screen.
            let auth = ctx.require_session()?;
            let req = request.to_owned_message();
            let account_id = self.webhook_account(&ctx, &req.bot_id).await?;
            let fields = self.check_webhook(&req.url, &req.description, &req.event_types)?;
            self.core.cache.rate_limit(&format!("dev-webhook-create:{}", auth.user_id), 30, Duration::from_secs(3600)).await?;
            let secret = secrets::generate(SecretKind::Webhook);
            let mut tx = self.core.db.begin().await?;
            // One at a time per account, so the limit cannot be raced past.
            lock_limit(&mut tx, "dev-webhooks", account_id).await?;
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM dev_webhooks WHERE account_id = $1").bind(account_id).fetch_one(&mut *tx).await?;
            ensure!(count < self.cfg.max_webhooks, precondition, "TOO_MANY_WEBHOOKS", "at most {} webhooks per account", self.cfg.max_webhooks);
            let row: WebhookRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "INSERT INTO dev_webhooks (account_id, url, description, event_types, secret_sealed, secret_hint) VALUES ($1, $2, $3, $4, $5, $6) RETURNING {WEBHOOK_COLUMNS}"
            )))
            .bind(account_id)
            .bind(&fields.url)
            .bind(&fields.description)
            .bind(&fields.event_types)
            .bind(self.seal_secret(&secret))
            .bind(secrets::hint(&secret))
            .fetch_one(&mut *tx)
            .await?;
            tx.commit().await?;
            self.subscribed.invalidate();
            ok(CreateWebhookResponse { webhook: row.to_proto().into(), secret, ..Default::default() })
        })
        .await
    }

    async fn update_webhook(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateWebhookRequest>,
    ) -> ServiceResult<UpdateWebhookResponse> {
        handle(async {
            let auth = ctx.require_session()?;
            let req = request.to_owned_message();
            let webhook = self.owned_webhook(&ctx, &req.webhook_id).await?;
            let fields = self.check_webhook(&req.url, &req.description, &req.event_types)?;
            self.core.cache.rate_limit(&format!("dev-webhook-update:{}", auth.user_id), 120, Duration::from_secs(3600)).await?;
            // Resuming — from a pause or from being disabled — starts with a clean record.
            let resumed = !req.paused && !webhook.is_active();
            let row: WebhookRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "UPDATE dev_webhooks SET url = $2, description = $3, event_types = $4, status = CASE WHEN $5 THEN 'paused' ELSE 'active' END,
                        disabled_reason = CASE WHEN $5 THEN disabled_reason ELSE '' END,
                        consecutive_failures = CASE WHEN $6 THEN 0 ELSE consecutive_failures END,
                        failing_since = CASE WHEN $6 THEN NULL ELSE failing_since END
                 WHERE id = $1 RETURNING {WEBHOOK_COLUMNS}"
            )))
            .bind(webhook.id)
            .bind(&fields.url)
            .bind(&fields.description)
            .bind(&fields.event_types)
            .bind(req.paused)
            .bind(resumed)
            .fetch_one(&self.core.db)
            .await?;
            self.subscribed.invalidate();
            ok(UpdateWebhookResponse { webhook: row.to_proto().into(), ..Default::default() })
        })
        .await
    }

    async fn get_webhook(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetWebhookRequest>,
    ) -> ServiceResult<GetWebhookResponse> {
        handle(async {
            ok(GetWebhookResponse {
                webhook: self.owned_webhook(&ctx, request.webhook_id).await?.to_proto().into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_webhooks(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListWebhooksRequest>,
    ) -> ServiceResult<ListWebhooksResponse> {
        handle(async {
            let account_id = self.webhook_account(&ctx, request.bot_id).await?;
            let rows: Vec<WebhookRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {WEBHOOK_COLUMNS} FROM dev_webhooks WHERE account_id = $1 ORDER BY id"
            )))
            .bind(account_id)
            .fetch_all(&self.core.db)
            .await?;
            ok(ListWebhooksResponse { webhooks: rows.iter().map(WebhookRow::to_proto).collect(), ..Default::default() })
        })
        .await
    }

    async fn delete_webhook(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteWebhookRequest>,
    ) -> ServiceResult<DeleteWebhookResponse> {
        handle(async {
            ctx.require_session()?;
            let webhook = self.owned_webhook(&ctx, request.webhook_id).await?;
            // The delivery log goes with it.
            sqlx::query("DELETE FROM dev_webhooks WHERE id = $1").bind(webhook.id).execute(&self.core.db).await?;
            self.subscribed.invalidate();
            ok(DeleteWebhookResponse::default())
        })
        .await
    }

    async fn rotate_webhook_secret(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RotateWebhookSecretRequest>,
    ) -> ServiceResult<RotateWebhookSecretResponse> {
        handle(async {
            ctx.require_session()?;
            let webhook = self.owned_webhook(&ctx, request.webhook_id).await?;
            self.core.cache.rate_limit(&format!("dev-webhook-rotate:{}", webhook.id), 10, Duration::from_secs(3600)).await?;
            let secret = secrets::generate(SecretKind::Webhook);
            let row: WebhookRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "UPDATE dev_webhooks SET previous_secret_sealed = secret_sealed, previous_secret_expires_at = $4, secret_sealed = $2, secret_hint = $3
                 WHERE id = $1 RETURNING {WEBHOOK_COLUMNS}"
            )))
            .bind(webhook.id)
            .bind(self.seal_secret(&secret))
            .bind(secrets::hint(&secret))
            .bind(Utc::now() + self.cfg.secret_overlap())
            .fetch_one(&self.core.db)
            .await?;
            ok(RotateWebhookSecretResponse { webhook: row.to_proto().into(), secret, ..Default::default() })
        })
        .await
    }

    async fn ping_webhook(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, PingWebhookRequest>,
    ) -> ServiceResult<PingWebhookResponse> {
        handle(async {
            ctx.require_session()?;
            let webhook = self.owned_webhook(&ctx, request.webhook_id).await?;
            // Per caller, not per webhook: each ping is a request to somebody's server.
            self.core
                .cache
                .rate_limit(&format!("dev-webhook-ping:{}", ctx.user_id()?), 20, Duration::from_secs(60))
                .await?;
            let delivery = self.ping(&webhook).await?;
            ok(PingWebhookResponse { delivery: delivery.to_proto(true).into(), ..Default::default() })
        })
        .await
    }

    async fn list_deliveries(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListDeliveriesRequest>,
    ) -> ServiceResult<ListDeliveriesResponse> {
        handle(async {
            let webhook = self.owned_webhook(&ctx, request.webhook_id).await?;
            let status = match request.status.as_known() {
                Some(DeliveryStatus::DELIVERY_STATUS_PENDING) => Some("pending"),
                Some(DeliveryStatus::DELIVERY_STATUS_SUCCEEDED) => Some("succeeded"),
                Some(DeliveryStatus::DELIVERY_STATUS_RETRYING) => Some("retrying"),
                Some(DeliveryStatus::DELIVERY_STATUS_FAILED) => Some("failed"),
                _ => None,
            };
            let page = Page::from_view(request.page.as_option(), MAX_PAGE_SIZE);
            let after: Option<Uuid> = page.decode()?;
            let mut rows: Vec<DeliveryRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {} FROM dev_webhook_deliveries
                 WHERE webhook_id = $1 AND ($2::text IS NULL OR status = $2) AND ($3::uuid IS NULL OR id < $3)
                   AND ($5::text[] IS NULL OR event_type = ANY($5))
                 ORDER BY id DESC LIMIT $4",
                delivery::DELIVERY_COLUMNS
            )))
            .bind(webhook.id)
            .bind(status)
            .bind(after)
            .bind(page.fetch())
            .bind(readable_event_types(ctx.auth()?))
            .fetch_all(&self.core.db)
            .await?;
            let info = page.finish(&mut rows, |row| row.id);
            ok(ListDeliveriesResponse {
                deliveries: rows.iter().map(|row| row.to_proto(false)).collect(),
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_delivery(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetDeliveryRequest>,
    ) -> ServiceResult<GetDeliveryResponse> {
        handle(async {
            let delivery = self.owned_delivery(&ctx, request.delivery_id).await?;
            // The payload is what the event type's scopes protect elsewhere.
            ensure!(
                may_read(ctx.auth()?, &delivery.event_type),
                denied,
                "INSUFFICIENT_SCOPE",
                "this token lacks the scopes of {} events",
                delivery.event_type
            );
            ok(GetDeliveryResponse { delivery: delivery.to_proto(true).into(), ..Default::default() })
        })
        .await
    }

    async fn replay_delivery(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ReplayDeliveryRequest>,
    ) -> ServiceResult<ReplayDeliveryResponse> {
        handle(async {
            ctx.require_session()?;
            let original = self.owned_delivery(&ctx, request.delivery_id).await?;
            let webhook = self.webhook(original.webhook_id).await?;
            ensure!(
                webhook.is_active(),
                precondition,
                "WEBHOOK_NOT_ACTIVE",
                "resume the webhook before replaying deliveries"
            );
            self.core
                .cache
                .rate_limit(&format!("dev-webhook-replay:{}", ctx.user_id()?), 60, Duration::from_secs(3600))
                .await?;
            let delivery = self.replay(&original).await?;
            ok(ReplayDeliveryResponse { delivery: delivery.to_proto(false).into(), ..Default::default() })
        })
        .await
    }
}

impl Dev {
    /// A delivery of a webhook the caller may manage.
    async fn owned_delivery(&self, ctx: &RequestContext, delivery_id: &str) -> AppResult<DeliveryRow> {
        let not_found = || AppError::not_found("DELIVERY_NOT_FOUND", "delivery not found");
        let row: DeliveryRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM dev_webhook_deliveries WHERE id = $1",
            delivery::DELIVERY_COLUMNS
        )))
        .bind(parse_id(delivery_id)?)
        .fetch_optional(&self.core.db)
        .await?
        .ok_or_else(not_found)?;
        // Somebody else's delivery does not exist as far as the caller is told.
        self.owned_webhook(ctx, &row.webhook_id.to_string()).await.map_err(|_| not_found())?;
        Ok(row)
    }
}

#[cfg(test)]
mod tests {
    use tokuly_core::api_tokens::is_scope;

    use super::*;
    use crate::scopes;

    #[test]
    fn the_catalogue_is_consistent() {
        let mut names: Vec<&str> = EVENT_TYPES.iter().map(|t| t.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), EVENT_TYPES.len(), "no type is listed twice");
        for event_type in EVENT_TYPES {
            assert!(
                !event_type.scopes.is_empty()
                    && event_type.scopes.iter().all(|scope| is_scope(scope) && scope.ends_with(":read")),
                "{}",
                event_type.name
            );
            assert!(
                event_type.scopes.iter().all(|scope| scopes::find(scope).is_some()),
                "{} needs scopes tokens can hold",
                event_type.name
            );
            assert!(!event_type.ja.is_empty() && !event_type.en.is_empty());
            if let Source::Topic { topic, accounts } = event_type.source {
                assert!(topic.split('.').count() == 3 && !accounts.is_empty(), "{}", event_type.name);
            }
        }
        assert!(event_type(PING).is_none(), "ping cannot be subscribed to");
        // The events the product promises.
        for wanted in [
            "friend.request_received",
            "chat.message_received",
            "notification.created",
            "place.alert",
            "channel.video_published",
        ] {
            assert!(event_type(wanted).is_some(), "{wanted}");
        }
        assert_eq!(event_type("place.alert").unwrap().scopes, ["geo:read"], "location events need the location scope");
        assert_eq!(
            event_type("notification.created").unwrap().scopes,
            ["notify:read", "chat:read", "geo:read"],
            "the inbox holds messages and place alerts"
        );
    }
}
