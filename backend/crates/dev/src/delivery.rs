//! Sending webhook deliveries: signing, the guard against requests into
//! private networks, retries, the delivery log and disabling dead endpoints.
//!
//! A delivery (`dev_webhook_deliveries`) is settled by attempts. Each
//! attempt is one `dev.webhook.deliver` job; a failed one schedules the
//! next (`next_attempt_at`, growing fourfold), and a periodic sweep queues
//! whatever is due — so a retry survives restarts and a lost job only
//! delays a delivery.
//!
//! The URL is the owner's, so the request must not be usable to reach
//! anything the owner could not reach themselves: only `https`, never a
//! redirect, no proxy, and the connection goes to an address that was
//! checked to be public *at connection time* (a custom resolver filters
//! what DNS returns, which also defeats names that re-resolve to an inside
//! address after validation).
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::Utc;
use hmac::{Hmac, Mac as _};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tokuly_core::{
    events,
    keys::secure_eq,
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::dev::{Delivery, DeliveryAttempt, DeliveryStatus};

use crate::{
    Dev, DevConfig,
    events::{DeliverJob, WebhookDisabled},
    webhooks::{PING, WEBHOOK_COLUMNS, WebhookRow},
};

pub const DELIVER_JOB: &str = "dev.webhook.deliver";
/// The signature header of every delivery.
pub const SIGNATURE_HEADER: &str = "tokuly-signature";
/// How much of an endpoint's answer is kept in the log.
const SNIPPET_CHARS: usize = 500;
/// How much of it is read at all.
const MAX_RESPONSE_BYTES: usize = 16 * 1024;

// ───────────────────────────── where a delivery may go ─────────────────────────────

/// Whether an address is on the public internet: not loopback, private,
/// link-local, carrier-grade NAT, multicast, documentation or otherwise
/// reserved — in either address family, including IPv4 addresses wrapped in
/// IPv6.
pub fn is_public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => is_public_v6(ip),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || a == 0
        // Carrier-grade NAT 100.64/10, IETF protocol assignments 192.0.0/24,
        // 6to4 relay anycast 192.88.99/24, benchmarking 198.18/15, reserved 240/4.
        || (a == 100 && (64..128).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 88 && c == 99)
        || (a == 198 && (b == 18 || b == 19))
        || a >= 240)
}

/// The IPv4 address an IPv6 address carries: IPv4-mapped (`::ffff:a.b.c.d`),
/// IPv4-compatible (`::a.b.c.d`), NAT64 (`64:ff9b::/96`) and 6to4
/// (`2002::/16`). Such an address reaches the IPv4 host, so it is judged by it.
fn embedded_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let segments = ip.segments();
    let low =
        || Ipv4Addr::new((segments[6] >> 8) as u8, segments[6] as u8, (segments[7] >> 8) as u8, segments[7] as u8);
    match segments {
        [0, 0, 0, 0, 0, 0xffff, _, _] | [0x64, 0xff9b, 0, 0, 0, 0, _, _] => Some(low()),
        [0, 0, 0, 0, 0, 0, high, _] if high != 0 => Some(low()),
        [0x2002, high, low_bits, ..] => {
            Some(Ipv4Addr::new((high >> 8) as u8, high as u8, (low_bits >> 8) as u8, low_bits as u8))
        }
        _ => None,
    }
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = embedded_v4(ip) {
        return is_public_v4(v4);
    }
    let segments = ip.segments();
    let first = segments[0];
    !(ip.is_unspecified()
        || ip.is_loopback()
        // Unique local fc00::/7, link-local fe80::/10, site-local fec0::/10, multicast ff00::/8.
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || (first & 0xffc0) == 0xfec0
        || (first & 0xff00) == 0xff00
        // Documentation 2001:db8::/32, Teredo 2001::/32, discard-only 100::/64.
        || (first == 0x2001 && segments[1] == 0x0db8)
        || (first == 0x2001 && segments[1] == 0)
        || (first == 0x0100 && segments[1..4] == [0, 0, 0]))
}

/// Checks a webhook URL as far as it can be without connecting. `Err`
/// completes the sentence "the URL …".
pub fn check_url(url: &str, allow_private: bool) -> Result<url::Url, &'static str> {
    let url = url::Url::parse(url).map_err(|_| "is not a valid URL")?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err("must not contain credentials");
    }
    if url.fragment().is_some() {
        return Err("must not have a fragment");
    }
    let host = url.host().ok_or("has no host")?;
    if allow_private {
        return if matches!(url.scheme(), "https" | "http") { Ok(url) } else { Err("must be http or https") };
    }
    if url.scheme() != "https" {
        return Err("must use https");
    }
    let public = match host {
        url::Host::Ipv4(ip) => is_public_v4(ip),
        url::Host::Ipv6(ip) => is_public_v6(ip),
        // Names are judged for real when they are resolved; this only turns
        // away the obvious at registration.
        url::Host::Domain(name) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            const INTERNAL: &[&str] = &[
                "localhost",
                "local",
                "internal",
                "lan",
                "home",
                "corp",
                "intranet",
                "test",
                "invalid",
                "example",
                "onion",
            ];
            name.rsplit_once('.').is_some_and(|(_, tld)| !INTERNAL.contains(&tld))
        }
    };
    if public { Ok(url) } else { Err("must point to a public host") }
}

/// Resolves names like the system does, then drops every address that is
/// not public. A name with no public address does not resolve.
struct PublicOnly;

impl reqwest::dns::Resolve for PublicOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let host = name.as_str().to_owned();
            let resolved: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|addr| is_public_address(addr.ip()))
                .collect();
            if resolved.is_empty() {
                return Err(format!("{host} has no public address").into());
            }
            Ok(Box::new(resolved.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// The HTTP client deliveries are sent with.
pub(crate) fn http_client(cfg: &DevConfig) -> anyhow::Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .user_agent("Tokuly-Webhooks/1.0")
        // A redirect could lead anywhere; a proxy would resolve names for us.
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(5).min(cfg.webhook_timeout()))
        .timeout(cfg.webhook_timeout());
    if !cfg.webhook_allow_private {
        builder = builder.dns_resolver(Arc::new(PublicOnly));
    }
    builder.build().map_err(|e| anyhow::anyhow!("webhook http client: {e}"))
}

// ───────────────────────────── signatures ─────────────────────────────

fn sign(secret: &str, timestamp: i64, body: &str) -> String {
    let mut mac =
        <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(format!("{timestamp}.{body}").as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// The `Tokuly-Signature` value for a body sent at `timestamp` (unix
/// seconds): `t=<timestamp>,v1=<hex>` with one `v1` per signing secret,
/// where each is HMAC-SHA256(secret, "<timestamp>.<body>").
pub fn signature_header(secrets: &[String], timestamp: i64, body: &str) -> String {
    let mut header = format!("t={timestamp}");
    for secret in secrets {
        header.push_str(",v1=");
        header.push_str(&sign(secret, timestamp, body));
    }
    header
}

/// What a receiver does with a delivery: checks that one of the header's
/// signatures is the secret's over this exact body, and that the timestamp
/// is no older (or newer) than `tolerance`, so a recorded delivery cannot
/// be replayed later.
pub fn verify_signature(secret: &str, header: &str, body: &str, tolerance: Duration, now: Time) -> bool {
    let mut timestamp = None;
    let mut signatures = Vec::new();
    for part in header.split(',') {
        match part.trim().split_once('=') {
            Some(("t", value)) => timestamp = value.parse::<i64>().ok(),
            Some(("v1", value)) => signatures.push(value),
            _ => {}
        }
    }
    let Some(timestamp) = timestamp else { return false };
    if (now.timestamp() - timestamp).unsigned_abs() > tolerance.as_secs() {
        return false;
    }
    let expected = sign(secret, timestamp, body);
    signatures.iter().any(|signature| secure_eq(signature.as_bytes(), expected.as_bytes()))
}

// ───────────────────────────── the log ─────────────────────────────

/// One try, as kept in the log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attempt {
    pub at: Time,
    /// 0 when the endpoint gave no answer.
    pub status_code: u16,
    pub latency_ms: i64,
    #[serde(default)]
    pub response_snippet: String,
    #[serde(default)]
    pub error: String,
}

impl Attempt {
    /// Any 2xx answer is a delivery.
    pub fn succeeded(&self) -> bool {
        (200..300).contains(&self.status_code)
    }

    fn to_proto(&self) -> DeliveryAttempt {
        DeliveryAttempt {
            at: ts(self.at),
            status_code: self.status_code as i32,
            latency_ms: self.latency_ms.min(i32::MAX as i64) as i32,
            response_snippet: self.response_snippet.clone(),
            error: self.error.clone(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct DeliveryRow {
    pub id: Uuid,
    pub webhook_id: Uuid,
    pub event_id: Uuid,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub status: String,
    pub attempts: serde_json::Value,
    pub attempt_count: i32,
    pub next_attempt_at: Option<Time>,
    pub replay_of: Option<Uuid>,
    pub created_at: Time,
    pub completed_at: Option<Time>,
}

pub(crate) const DELIVERY_COLUMNS: &str = "id, webhook_id, event_id, event_type, payload, status, attempts, attempt_count, next_attempt_at, replay_of, created_at, completed_at";

impl DeliveryRow {
    fn attempts(&self) -> Vec<Attempt> {
        serde_json::from_value(self.attempts.clone()).unwrap_or_default()
    }

    /// `full` adds the payload and every attempt; otherwise just the last one.
    pub(crate) fn to_proto(&self, full: bool) -> Delivery {
        let attempts = self.attempts();
        let shown =
            if full { attempts.as_slice() } else { attempts.last().map(std::slice::from_ref).unwrap_or_default() };
        let status = match self.status.as_str() {
            "pending" => DeliveryStatus::DELIVERY_STATUS_PENDING,
            "succeeded" => DeliveryStatus::DELIVERY_STATUS_SUCCEEDED,
            "retrying" => DeliveryStatus::DELIVERY_STATUS_RETRYING,
            _ => DeliveryStatus::DELIVERY_STATUS_FAILED,
        };
        Delivery {
            id: self.id.to_string(),
            webhook_id: self.webhook_id.to_string(),
            event_id: self.event_id.to_string(),
            event_type: self.event_type.clone(),
            status: status.into(),
            attempt_count: self.attempt_count,
            created_at: ts(self.created_at),
            // While a delivery is waiting for its first attempt this is only the sweep's safety net.
            next_attempt_at: opt_ts(self.next_attempt_at.filter(|_| self.status == "retrying")),
            completed_at: opt_ts(self.completed_at),
            replay_of: self.replay_of.map(|id| id.to_string()).unwrap_or_default(),
            attempts: shown.iter().map(Attempt::to_proto).collect(),
            payload: if full { self.payload.to_string() } else { String::new() },
            ..Default::default()
        }
    }
}

/// The body of a delivery: `{ id, type, time, data }`.
pub(crate) fn envelope(event_id: Uuid, event_type: &str, time: Time, data: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "id": event_id, "type": event_type, "time": time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true), "data": data })
}

fn truncate_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// Why a request got no answer, in words fit for the delivery log.
fn describe_failure(error: &reqwest::Error) -> String {
    use std::error::Error as _;
    if error.is_timeout() {
        return "timeout".to_owned();
    }
    if error.is_redirect() {
        return "redirects are not followed".to_owned();
    }
    // The resolver's refusal sits at the bottom of the chain.
    let mut source = error.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if text.contains("has no public address") {
            return "address not allowed: the host does not resolve to a public address".to_owned();
        }
        source = cause.source();
    }
    if error.is_connect() { "connection failed".to_owned() } else { "request failed".to_owned() }
}

impl Dev {
    /// Makes one HTTP attempt at delivering `payload` to `webhook`.
    async fn send(
        &self,
        webhook: &WebhookRow,
        delivery_id: Uuid,
        event_type: &str,
        payload: &serde_json::Value,
    ) -> Attempt {
        let at = Utc::now();
        let failed = |error: &str| Attempt {
            at,
            status_code: 0,
            latency_ms: 0,
            response_snippet: String::new(),
            error: error.to_owned(),
        };
        // The rules may be stricter now than when the URL was registered.
        let url = match check_url(&webhook.url, self.cfg.webhook_allow_private) {
            Ok(url) => url,
            Err(_) => return failed("address not allowed"),
        };
        let body = payload.to_string();
        let signature = signature_header(&webhook.signing_secrets(self), at.timestamp(), &body);
        let started = Instant::now();
        let result = self
            .webhook_http
            .post(url)
            .header("content-type", "application/json")
            .header(SIGNATURE_HEADER, signature)
            .header("tokuly-event", event_type)
            .header("tokuly-delivery", delivery_id.to_string())
            .body(body)
            .send()
            .await;
        let latency_ms = started.elapsed().as_millis() as i64;
        match result {
            Ok(mut response) => {
                let status_code = response.status().as_u16();
                let mut bytes = Vec::new();
                // The other end is not ours: read a bounded amount, and do not wait for more than the log shows.
                while bytes.len() < MAX_RESPONSE_BYTES {
                    match response.chunk().await {
                        Ok(Some(chunk)) => bytes.extend_from_slice(&chunk),
                        _ => break,
                    }
                }
                let error = if (300..400).contains(&status_code) {
                    "redirects are not followed".to_owned()
                } else {
                    String::new()
                };
                Attempt {
                    at,
                    status_code,
                    latency_ms,
                    response_snippet: truncate_chars(String::from_utf8_lossy(&bytes).trim(), SNIPPET_CHARS),
                    error,
                }
            }
            Err(e) => Attempt { latency_ms, ..failed(&describe_failure(&e)) },
        }
    }

    async fn delivery(&self, id: Uuid) -> AppResult<DeliveryRow> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {DELIVERY_COLUMNS} FROM dev_webhook_deliveries WHERE id = $1"
        )))
        .bind(id)
        .fetch_one(&self.core.db)
        .await?)
    }

    /// Stores a delivery to be made and queues its first attempt.
    pub(crate) async fn queue_delivery(
        &self,
        webhook_id: Uuid,
        event_id: Uuid,
        event_type: &str,
        payload: &serde_json::Value,
        replay_of: Option<Uuid>,
    ) -> AppResult<Option<Uuid>> {
        // `next_attempt_at` is the safety net: if the job below is lost, the sweep queues another.
        let id: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO dev_webhook_deliveries (webhook_id, event_id, event_type, payload, replay_of, next_attempt_at)
             VALUES ($1, $2, $3, $4, $5, now() + interval '2 minutes')
             ON CONFLICT (webhook_id, event_id, event_type) WHERE replay_of IS NULL DO NOTHING RETURNING id",
        )
        .bind(webhook_id)
        .bind(event_id)
        .bind(event_type)
        .bind(payload)
        .bind(replay_of)
        .fetch_optional(&self.core.db)
        .await?;
        if let Some(delivery_id) = id {
            if let Err(e) = self.core.jobs.enqueue(DELIVER_JOB, &DeliverJob { delivery_id }).await {
                tracing::warn!(%delivery_id, error = %e, "webhook delivery not queued; the sweep will pick it up");
            }
        }
        Ok(id)
    }

    /// Makes the next attempt at a delivery, if one is due and nobody else
    /// is making it. This is the body of the `dev.webhook.deliver` job.
    /// Returns the delivery as it stands afterwards, or `None` when there
    /// was nothing to do.
    pub async fn deliver(&self, delivery_id: Uuid) -> AppResult<Option<Delivery>> {
        let claimed: Option<DeliveryRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE dev_webhook_deliveries SET claimed_at = now()
             WHERE id = $1 AND status IN ('pending', 'retrying') AND (claimed_at IS NULL OR claimed_at < now() - interval '5 minutes')
             RETURNING {DELIVERY_COLUMNS}"
        )))
        .bind(delivery_id)
        .fetch_optional(&self.core.db)
        .await?;
        let Some(delivery) = claimed else { return Ok(None) };
        let webhook: WebhookRow =
            sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {WEBHOOK_COLUMNS} FROM dev_webhooks WHERE id = $1")))
                .bind(delivery.webhook_id)
                .fetch_one(&self.core.db)
                .await?;
        if !webhook.is_active() {
            // Paused or disabled since the delivery was queued: it is not sent.
            let note = Attempt {
                at: Utc::now(),
                status_code: 0,
                latency_ms: 0,
                response_snippet: String::new(),
                error: format!("not sent: the webhook is {}", webhook.status),
            };
            return Ok(Some(self.settle(&delivery, &webhook, note, true).await?.to_proto(true)));
        }
        let attempt = self.send(&webhook, delivery.id, &delivery.event_type, &delivery.payload).await;
        Ok(Some(self.settle(&delivery, &webhook, attempt, false).await?.to_proto(true)))
    }

    /// Records an attempt and decides what happens to the delivery and the
    /// webhook. `give_up` settles the delivery as failed without counting
    /// against the endpoint.
    ///
    /// Attempts at different deliveries of one webhook finish at the same
    /// time, and one of them may disable it. They are therefore settled one
    /// after the other, each holding the webhook's row lock and reading its
    /// state afresh: the webhook is disabled — and its owner told — exactly
    /// once, and an attempt that was in flight meanwhile is recorded but not
    /// retried. Everything inside runs on this one transaction.
    async fn settle(
        &self,
        delivery: &DeliveryRow,
        webhook: &WebhookRow,
        attempt: Attempt,
        give_up: bool,
    ) -> AppResult<DeliveryRow> {
        let succeeded = !give_up && attempt.succeeded();
        let attempt_count = delivery.attempt_count + i32::from(!give_up);
        // A ping is a single try; it says how the endpoint answers, nothing more.
        let is_ping = delivery.event_type == PING;
        let attempt_json =
            serde_json::to_value(&attempt).map_err(|e| AppError::internal(anyhow::anyhow!("attempt encode: {e}")))?;
        let mut tx = self.core.db.begin().await?;
        // `None`: the webhook was deleted while the attempt was in flight, and its log with it.
        let state: Option<(String, i32)> =
            sqlx::query_as("SELECT status, consecutive_failures FROM dev_webhooks WHERE id = $1 FOR UPDATE")
                .bind(webhook.id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some((webhook_status, failures)) = state else {
            return Ok(DeliveryRow { status: "failed".to_owned(), ..delivery.clone() });
        };
        let counts = !give_up && !is_ping;
        let mut disabled = false;
        if counts && succeeded {
            sqlx::query("UPDATE dev_webhooks SET consecutive_failures = 0, failing_since = NULL, last_success_at = now() WHERE id = $1").bind(webhook.id).execute(&mut *tx).await?;
        } else if counts {
            // Disabled once it has failed often enough for long enough.
            let now_disabled: bool = sqlx::query_scalar(
                "UPDATE dev_webhooks SET consecutive_failures = consecutive_failures + 1, failing_since = COALESCE(failing_since, now()), last_failure_at = now(),
                        status = CASE WHEN status = 'active' AND consecutive_failures + 1 >= $2 AND COALESCE(failing_since, now()) <= now() - make_interval(hours => $3)
                                      THEN 'disabled' ELSE status END
                 WHERE id = $1
                 RETURNING status = 'disabled'",
            )
            .bind(webhook.id)
            .bind(self.cfg.webhook_disable_after_failures)
            .bind(self.cfg.webhook_disable_after_hours as i32)
            .fetch_one(&mut *tx)
            .await?;
            disabled = now_disabled && webhook_status == "active";
        }
        // No retries towards a webhook that is not active (any more).
        let exhausted = give_up
            || is_ping
            || disabled
            || webhook_status != "active"
            || attempt_count >= self.cfg.webhook_max_attempts;
        let (status, next_attempt_at) = match (succeeded, exhausted) {
            (true, _) => ("succeeded", None),
            (false, true) => ("failed", None),
            (false, false) => ("retrying", Some(Utc::now() + self.cfg.retry_delay(attempt_count))),
        };
        sqlx::query(
            "UPDATE dev_webhook_deliveries SET status = $2, attempts = attempts || $3::jsonb, attempt_count = $4, next_attempt_at = $5, claimed_at = NULL,
                    completed_at = CASE WHEN $5::timestamptz IS NULL THEN now() END
             WHERE id = $1",
        )
        .bind(delivery.id)
        .bind(status)
        .bind(serde_json::Value::Array(vec![attempt_json]))
        .bind(attempt_count)
        .bind(next_attempt_at)
        .execute(&mut *tx)
        .await?;
        if disabled {
            let reason =
                format!("{} attempts in a row failed; the last error was: {}", failures + 1, attempt_summary(&attempt));
            sqlx::query("UPDATE dev_webhooks SET disabled_reason = $2 WHERE id = $1")
                .bind(webhook.id)
                .bind(&reason)
                .execute(&mut *tx)
                .await?;
            // What is still waiting will not be sent. Attempts in flight settle themselves.
            sqlx::query(
                "UPDATE dev_webhook_deliveries SET status = 'failed', next_attempt_at = NULL, completed_at = now()
                 WHERE webhook_id = $1 AND status IN ('pending', 'retrying') AND claimed_at IS NULL",
            )
            .bind(webhook.id)
            .execute(&mut *tx)
            .await?;
            self.announce_disabled(&mut tx, webhook).await?;
        }
        let settled: DeliveryRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {DELIVERY_COLUMNS} FROM dev_webhook_deliveries WHERE id = $1"
        )))
        .bind(delivery.id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        if disabled {
            self.subscribed.invalidate();
        }
        Ok(settled)
    }

    /// Tells the person behind a webhook that it was switched off.
    async fn announce_disabled(&self, tx: &mut sqlx::PgConnection, webhook: &WebhookRow) -> AppResult<()> {
        // A bot's webhook is its owner's to fix.
        let owner: Option<Uuid> = sqlx::query_scalar("SELECT owner_id FROM dev_bots WHERE user_id = $1")
            .bind(webhook.account_id)
            .fetch_optional(&mut *tx)
            .await?;
        let host =
            url::Url::parse(&webhook.url).ok().and_then(|url| url.host_str().map(str::to_owned)).unwrap_or_default();
        let notice = NotificationRequest::new(
            owner.unwrap_or(webhook.account_id),
            "dev.webhook_disabled",
            Localized::new("Webhookを停止しました", "A webhook was disabled"),
            Localized::new(
                format!("{host} への配信が失敗し続けたため、Webhookを停止しました。エンドポイントを確認して再開してください！"),
                format!("Deliveries to {host} kept failing, so the webhook was disabled. Check the endpoint and resume it."),
            ),
            format!("/settings/developer/webhooks/{}", webhook.id),
        )
        .data(serde_json::json!({ "webhook_id": webhook.id }));
        outbound::notify(&mut *tx, &notice).await?;
        events::emit(
            &mut *tx,
            "dev.webhook.disabled",
            None,
            &WebhookDisabled { webhook_id: webhook.id, account_id: webhook.account_id },
        )
        .await
    }

    /// Sends a `ping` now and returns how the endpoint answered.
    pub(crate) async fn ping(&self, webhook: &WebhookRow) -> AppResult<DeliveryRow> {
        let event_id = new_id();
        let payload = envelope(event_id, PING, Utc::now(), serde_json::json!({ "webhook_id": webhook.id }));
        let delivery: DeliveryRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO dev_webhook_deliveries (webhook_id, event_id, event_type, payload, claimed_at) VALUES ($1, $2, $3, $4, now()) RETURNING {DELIVERY_COLUMNS}"
        )))
        .bind(webhook.id)
        .bind(event_id)
        .bind(PING)
        .bind(&payload)
        .fetch_one(&self.core.db)
        .await?;
        let attempt = self.send(webhook, delivery.id, PING, &payload).await;
        self.settle(&delivery, webhook, attempt, false).await
    }

    /// Queues the event of an earlier delivery again, as a new delivery.
    pub(crate) async fn replay(&self, original: &DeliveryRow) -> AppResult<DeliveryRow> {
        let id = self
            .queue_delivery(
                original.webhook_id,
                original.event_id,
                &original.event_type,
                &original.payload,
                Some(original.replay_of.unwrap_or(original.id)),
            )
            .await?
            .ok_or_else(|| AppError::internal(anyhow::anyhow!("replay was not stored")))?;
        self.delivery(id).await
    }

    /// Queues every delivery whose next attempt is due. Run periodically;
    /// returns how many were queued.
    pub async fn queue_due_deliveries(&self) -> AppResult<usize> {
        // Moving the due time forward is the lease: a job that gets lost is queued again later.
        let due: Vec<Uuid> = sqlx::query_scalar(
            "UPDATE dev_webhook_deliveries SET next_attempt_at = now() + interval '5 minutes'
             WHERE id IN (SELECT id FROM dev_webhook_deliveries WHERE next_attempt_at <= now() ORDER BY next_attempt_at LIMIT 500 FOR UPDATE SKIP LOCKED)
             RETURNING id",
        )
        .fetch_all(&self.core.db)
        .await?;
        for delivery_id in &due {
            self.core.jobs.enqueue(DELIVER_JOB, &DeliverJob { delivery_id: *delivery_id }).await?;
        }
        Ok(due.len())
    }

    /// Drops log entries older than the retention.
    pub(crate) async fn purge_old_deliveries(&self) -> AppResult<u64> {
        let done = sqlx::query("DELETE FROM dev_webhook_deliveries WHERE created_at < now() - make_interval(days => $1) AND next_attempt_at IS NULL")
            .bind(self.cfg.webhook_log_days as i32)
            .execute(&self.core.db)
            .await?;
        Ok(done.rows_affected())
    }
}

fn attempt_summary(attempt: &Attempt) -> String {
    if attempt.error.is_empty() { format!("HTTP {}", attempt.status_code) } else { attempt.error.clone() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_public_addresses_may_be_reached() {
        for public in [
            "8.8.8.8",
            "203.0.114.1",
            "1.1.1.1",
            "2606:4700:4700::1111",
            "2001:4860:4860::8888",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
        ] {
            assert!(is_public_address(public.parse().unwrap()), "{public}");
        }
        for private in [
            "127.0.0.1",
            "127.8.9.10",
            "10.0.0.5",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "0.0.0.0",
            "0.1.2.3",
            "100.64.0.1",
            "100.127.255.254",
            "192.0.0.8",
            "192.0.2.1",
            "198.18.0.1",
            "198.51.100.7",
            "203.0.113.9",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
            "::1",
            "::",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "ff02::1",
            "2001:db8::1",
            // Tunnels and other special ranges.
            "192.88.99.1",
            "2001::1",
            "fec0::1",
            "100::1",
            // IPv4 hiding in IPv6: 6to4, then mapped, compatible and NAT64.
            "2002:7f00:1::1",
            "2002:a9fe:a9fe::1",
            "2002:c0a8:101::",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::ffff:169.254.169.254",
            "::127.0.0.1",
            "64:ff9b::7f00:1",
            "64:ff9b::a00:1",
        ] {
            assert!(!is_public_address(private.parse().unwrap()), "{private}");
        }
    }

    #[test]
    fn webhook_urls_are_https_to_public_hosts() {
        for good in ["https://hooks.example.com/tokuly", "https://example.org:8443/a?b=c", "https://8.8.8.8/hook"] {
            assert!(check_url(good, false).is_ok(), "{good}");
        }
        for (bad, why) in [
            ("http://hooks.example.com/tokuly", "must use https"),
            ("ftp://hooks.example.com/", "must use https"),
            ("https://user:pw@hooks.example.com/", "must not contain credentials"),
            ("https://hooks.example.com/#x", "must not have a fragment"),
            ("https://localhost/hook", "must point to a public host"),
            ("https://intranet/hook", "must point to a public host"),
            ("https://db.internal/hook", "must point to a public host"),
            ("https://printer.local/hook", "must point to a public host"),
            ("https://127.0.0.1/hook", "must point to a public host"),
            ("https://169.254.169.254/latest/meta-data", "must point to a public host"),
            ("https://[::1]/hook", "must point to a public host"),
            ("https://[::ffff:10.0.0.1]/hook", "must point to a public host"),
            ("https://10.1.2.3:8443/hook", "must point to a public host"),
            ("not a url", "is not a valid URL"),
        ] {
            assert_eq!(check_url(bad, false).unwrap_err(), why, "{bad}");
        }
        // Development: anything over http(s).
        assert!(check_url("http://127.0.0.1:9000/hook", true).is_ok());
        assert!(check_url("file:///etc/passwd", true).is_err());
    }

    #[tokio::test]
    async fn names_without_a_public_address_do_not_resolve() {
        use reqwest::dns::Resolve as _;
        let refused = PublicOnly.resolve("localhost".parse().unwrap()).await;
        assert!(refused.is_err_and(|e| e.to_string().contains("has no public address")));
    }

    #[test]
    fn signatures_bind_the_body_to_a_time() {
        let now = Utc::now();
        let secret = "tkl_whs_current".to_owned();
        let body = r#"{"id":"1","type":"ping"}"#;
        let header = signature_header(std::slice::from_ref(&secret), now.timestamp(), body);
        assert!(header.starts_with(&format!("t={},v1=", now.timestamp())));
        let tolerance = Duration::from_secs(300);
        assert!(verify_signature(&secret, &header, body, tolerance, now));
        assert!(!verify_signature("tkl_whs_other", &header, body, tolerance, now), "another secret");
        assert!(!verify_signature(&secret, &header, r#"{"id":"2","type":"ping"}"#, tolerance, now), "another body");
        // A captured delivery is worthless later.
        assert!(!verify_signature(&secret, &header, body, tolerance, now + chrono::Duration::seconds(301)));
        // Changing the timestamp to make it fresh breaks the signature.
        let forged = header.replace(&format!("t={}", now.timestamp()), &format!("t={}", now.timestamp() + 600));
        assert!(!verify_signature(&secret, &forged, body, tolerance, now + chrono::Duration::seconds(600)));
        assert!(
            !verify_signature(&secret, "v1=abc", body, tolerance, now)
                && !verify_signature(&secret, "", body, tolerance, now)
        );
    }

    #[test]
    fn a_rotated_secret_is_signed_alongside_the_new_one() {
        let now = Utc::now();
        let secrets = ["tkl_whs_new".to_owned(), "tkl_whs_old".to_owned()];
        let header = signature_header(&secrets, now.timestamp(), "{}");
        assert_eq!(header.matches("v1=").count(), 2);
        for secret in &secrets {
            assert!(verify_signature(secret, &header, "{}", Duration::from_secs(60), now), "{secret}");
        }
    }

    #[test]
    fn the_signature_is_plain_hmac_sha256_over_timestamp_dot_body() {
        // printf '%s' '1700000000.{}' | openssl dgst -sha256 -hmac secret
        assert_eq!(
            sign("secret", 1_700_000_000, "{}"),
            "b8569b78799ff9e3cbff0fc2d63a33a2b57f3282abd07c37ae5e8e7d79a5f163"
        );
        assert_ne!(sign("secret", 1_700_000_000, "{}"), sign("secret", 1_700_000_001, "{}"));
    }

    #[test]
    fn the_envelope_has_id_type_time_and_data() {
        let id = Uuid::now_v7();
        let body = envelope(id, "friend.added", Utc::now(), serde_json::json!({ "user_id": "u" }));
        let keys: Vec<&str> = body.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, ["data", "id", "time", "type"]);
        assert_eq!(body["id"], id.to_string());
        assert!(chrono::DateTime::parse_from_rfc3339(body["time"].as_str().unwrap()).is_ok());
    }
}
