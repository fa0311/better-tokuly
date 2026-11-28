//! `SystemService`: server stats, the audit log and the dead letters.
//! Every RPC needs the `admin` role.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::{
    bus::STREAM_DEAD,
    dead_letters::{self, Buried},
    prelude::*,
};
use tokuly_proto::admin::*;

use crate::{
    Admin,
    audit::{Audit, AuditFilter},
    staff::Staff,
};

/// Most dead letters one `ListDeadLetters` returns.
const LIST_LIMIT: usize = 200;
/// Most dead letters one `RedriveDeadLetters` sends round again.
const REDRIVE_LIMIT: usize = 1_000;
/// A consumer name is short; a longer prefix matches nothing.
const MAX_PREFIX_CHARS: usize = 100;

impl SystemService for Admin {
    async fn get_server_stats(&self, ctx: RequestContext, _request: ServiceRequest<'_, GetServerStatsRequest>) -> ServiceResult<GetServerStatsResponse> {
        handle(async {
            let staff = Staff::admin(&ctx)?;
            // The snapshot runs several full-table counts; keep a dashboard
            // left on auto-refresh from hammering the database.
            self.core.cache.rate_limit(&format!("admin-stats:{}", staff.id), 30, Duration::from_secs(60)).await?;
            ok(self.server_stats().await?)
        })
        .await
    }

    async fn list_audit_log(&self, ctx: RequestContext, request: ServiceRequest<'_, ListAuditLogRequest>) -> ServiceResult<ListAuditLogResponse> {
        handle(async {
            Staff::admin(&ctx)?;
            let request = request.to_owned_message();
            let filter = AuditFilter {
                actor_id: parse_opt_id(&request.actor_id)?,
                action: request.action.trim().to_owned(),
                target_type: request.target_type.trim().to_owned(),
                target_id: request.target_id.trim().to_owned(),
                target_user_id: parse_opt_id(&request.target_user_id)?,
                since: request.since.as_option().map(from_ts),
                until: request.until.as_option().map(from_ts),
            };
            if let (Some(since), Some(until)) = (filter.since, filter.until) {
                ensure!(since < until, invalid, "TIME_RANGE_INVALID", "`since` must be before `until`");
            }
            let page = Page::from_request(request.page.as_option(), 200);
            let mut rows = self.list_audit(&filter, &page).await?;
            let info = page.finish(&mut rows, |r| r.id);
            ok(ListAuditLogResponse { entries: self.audit_entries(&filter, rows).await?, page: info.into(), ..Default::default() })
        })
        .await
    }
    async fn list_dead_letters(&self, ctx: RequestContext, request: ServiceRequest<'_, ListDeadLettersRequest>) -> ServiceResult<ListDeadLettersResponse> {
        handle(async {
            let staff = Staff::admin(&ctx)?;
            // Every listing reads the stream message by message.
            self.core.cache.rate_limit(&format!("admin-dead-letters:{}", staff.id), 30, Duration::from_secs(60)).await?;
            let request = request.to_owned_message();
            let prefix = consumer_prefix(&request.consumer_prefix)?;
            let limit = usize::try_from(request.limit).ok().filter(|n| (1..=LIST_LIMIT).contains(n)).unwrap_or(LIST_LIMIT);
            let buried = dead_letters::list(&self.core.bus, prefix, limit).await?;
            ok(ListDeadLettersResponse {
                letters: buried.into_iter().map(dead_letter_to_proto).collect(),
                total: self.dead_letter_count().await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn redrive_dead_letters(&self, ctx: RequestContext, request: ServiceRequest<'_, RedriveDeadLettersRequest>) -> ServiceResult<RedriveDeadLettersResponse> {
        handle(async {
            let staff = Staff::admin(&ctx)?;
            self.core.cache.rate_limit(&format!("admin-dead-letters-redrive:{}", staff.id), 10, Duration::from_secs(60)).await?;
            let request = request.to_owned_message();
            let prefix = consumer_prefix(&request.consumer_prefix)?;
            let done = dead_letters::redrive(&self.core.bus, prefix, REDRIVE_LIMIT).await?;
            Audit::by(&staff, "dead_letters.redriven")
                .dead_letters(prefix.unwrap_or_default())
                .after(serde_json::json!({ "sent": done.sent, "failed": done.failed }))
                .record(&self.core.db)
                .await?;
            ok(RedriveDeadLettersResponse { sent: done.sent as i64, failed: done.failed as i64, ..Default::default() })
        })
        .await
    }
}

impl Admin {
    /// Messages on the `DEAD` stream.
    async fn dead_letter_count(&self) -> AppResult<i64> {
        let unavailable = |e: String| AppError::unavailable("BUS_UNAVAILABLE", format!("dead letters cannot be read: {e}"));
        let bus = &self.core.bus;
        let mut stream = bus.js.get_stream(bus.stream_name(STREAM_DEAD)).await.map_err(|e| unavailable(e.to_string()))?;
        let messages = stream.info().await.map_err(|e| unavailable(e.to_string()))?.state.messages;
        Ok(i64::try_from(messages).unwrap_or(i64::MAX))
    }
}

/// The consumer filter of a request: `None` for every consumer.
fn consumer_prefix(raw: &str) -> AppResult<Option<&str>> {
    let prefix = raw.trim();
    ensure!(prefix.chars().count() <= MAX_PREFIX_CHARS, invalid, "CONSUMER_PREFIX_TOO_LONG", "a consumer name is at most {MAX_PREFIX_CHARS} characters");
    Ok((!prefix.is_empty()).then_some(prefix))
}

/// A dead letter as staff see it: where and why, not what it carried. An
/// event names its ID and time; a job nothing more.
fn dead_letter_to_proto(Buried { sequence, letter }: Buried) -> DeadLetter {
    let event = letter.subject.starts_with("evt.").then(|| serde_json::from_value::<tokuly_core::events::Event>(letter.payload.clone()).ok()).flatten();
    DeadLetter {
        sequence: i64::try_from(sequence).unwrap_or(i64::MAX),
        consumer: letter.consumer,
        subject: letter.subject,
        error: letter.error,
        deliveries: letter.deliveries,
        died_at: ts(letter.died_at),
        event_id: event.as_ref().map(|e| e.id.to_string()).unwrap_or_default(),
        occurred_at: opt_ts(event.map(|e| e.occurred_at)),
        ..Default::default()
    }
}

