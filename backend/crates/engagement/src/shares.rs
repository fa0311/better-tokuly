//! Share tracking: how often a subject was shared, by target.
//!
//! PostgreSQL keeps the counts (`subject_stats.share_count`, `share_counts`
//! per target). Single shares are not rows: each one is an analytics event
//! (`engagement.share`) and a domain event (`engagement.share.recorded`).
use std::time::Duration;

use tokuly_core::{analytics::AnalyticsEvent, events, prelude::*};
use tokuly_proto::engagement::{ShareTarget, ShareTargetCount};

use crate::{
    Engagement,
    effects::Effects,
    events::{SHARE_RECORDED, ShareRecorded},
    stats::{self, Counter},
    subjects::Subject,
};

/// Shares one signed-in user may record per minute.
const SHARES_PER_MINUTE: u32 = 60;
/// Shares guests behind one address may record per minute.
const GUEST_SHARES_PER_MINUTE: u32 = 30;

/// The share target of a request; `SHARE_TARGET_INVALID` when unset or unknown.
pub(crate) fn requested_target(value: buffa::EnumValue<ShareTarget>) -> AppResult<ShareTarget> {
    match value.as_known() {
        Some(target) if target != ShareTarget::SHARE_TARGET_UNSPECIFIED => Ok(target),
        _ => Err(AppError::invalid("SHARE_TARGET_INVALID", "say how it was shared")),
    }
}

impl Engagement {
    /// Records that a subject was shared (`EngagementService.RecordShare`)
    /// and returns its new share count. `client_ip` rate-limits guests.
    pub(crate) async fn record_share(&self, user_id: Option<Uuid>, client_ip: &str, subject: &Subject, target: ShareTarget) -> AppResult<i64> {
        let cache = &self.core.cache;
        match user_id {
            Some(user_id) => cache.rate_limit(&format!("engagement:share:{user_id}"), SHARES_PER_MINUTE, Duration::from_secs(60)).await?,
            None => cache.rate_limit(&format!("engagement:share:ip:{client_ip}"), GUEST_SHARES_PER_MINUTE, Duration::from_secs(60)).await?,
        }
        let key = subject.key;
        let mut tx = self.core.db.begin().await?;
        let mut effects = Effects::default();
        stats::add(&mut tx, key, Some(subject.info.owner_id), Counter::Share, 1).await?;
        effects.count(key, Counter::Share, 1);
        sqlx::query(
            "INSERT INTO share_counts (subject_type, subject_id, target, count) VALUES ($1, $2, $3, 1)
             ON CONFLICT (subject_type, subject_id, target) DO UPDATE SET count = share_counts.count + 1",
        )
        .bind(key.kind)
        .bind(key.id)
        .bind(target as i32)
        .execute(&mut *tx)
        .await?;
        let share_count: i64 = sqlx::query_scalar("SELECT share_count FROM subject_stats WHERE subject_type = $1 AND subject_id = $2")
            .bind(key.kind)
            .bind(key.id)
            .fetch_one(&mut *tx)
            .await?;
        let event = ShareRecorded {
            user_id,
            subject_type: key.kind,
            subject_id: key.id,
            subject_owner_id: subject.info.owner_id,
            target: target as i32,
        };
        events::emit(&mut *tx, SHARE_RECORDED, user_id, &event).await?;
        tx.commit().await?;
        self.apply(effects).await;

        let mut tracked = AnalyticsEvent::new("engagement.share")
            .user(user_id)
            .subject("subject", key.id)
            .props(serde_json::json!({ "subject_type": key.kind, "target": target as i32 }));
        if let Some(owner_id) = subject.owner() {
            tracked = tracked.owner(owner_id);
        }
        self.core.analytics.track(tracked).await;
        Ok(share_count)
    }

    /// How often a subject was shared, by target (`EngagementService.GetShareStats`).
    pub(crate) async fn share_stats(&self, subject: &Subject) -> AppResult<(i64, Vec<ShareTargetCount>)> {
        let rows: Vec<(i32, i64)> = sqlx::query_as(
            "SELECT target, count FROM share_counts WHERE subject_type = $1 AND subject_id = $2 AND count > 0 ORDER BY target",
        )
        .bind(subject.key.kind)
        .bind(subject.key.id)
        .fetch_all(&self.core.db)
        .await?;
        let total = rows.iter().map(|(_, count)| count).sum();
        let targets = rows.into_iter().map(|(target, count)| ShareTargetCount { target: target.into(), count, ..Default::default() }).collect();
        Ok((total, targets))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_share_needs_a_known_target() {
        assert_eq!(requested_target(ShareTarget::SHARE_TARGET_CHAT.into()).unwrap(), ShareTarget::SHARE_TARGET_CHAT);
        assert_eq!(requested_target(ShareTarget::SHARE_TARGET_UNSPECIFIED.into()).unwrap_err().reason, "SHARE_TARGET_INVALID");
        assert_eq!(requested_target(buffa::EnumValue::from(99)).unwrap_err().reason, "SHARE_TARGET_INVALID");
    }
}
