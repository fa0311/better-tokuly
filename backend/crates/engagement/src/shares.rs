//! Share tracking: how often a subject was shared, by target.
//!
//! PostgreSQL keeps the counts (`subject_stats.share_count`, `share_counts`
//! per target). Single shares are not rows: each one is an analytics event
//! (`engagement.share`) and a domain event (`engagement.share.recorded`).
use std::{net::IpAddr, time::Duration};

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
/// How long sharing the same subject the same way again is not counted again.
const REPEAT_WINDOW: Duration = Duration::from_secs(3600);

/// How guests behind an address are told apart, without keeping the
/// address: an HMAC (under a key derived from the server secret) of the
/// address, or of its /64 network for IPv6, where one connection usually
/// has a whole /64 to pick from.
pub(crate) fn guest_key(core: &Core, client_ip: &str) -> String {
    guest_key_with(&core.keys, client_ip)
}

fn guest_key_with(keys: &tokuly_core::keys::Keys, client_ip: &str) -> String {
    let network = match client_ip.parse::<IpAddr>() {
        Ok(IpAddr::V6(address)) => match address.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => {
                let segments = address.segments();
                format!("{:x}:{:x}:{:x}:{:x}::/64", segments[0], segments[1], segments[2], segments[3])
            }
        },
        Ok(IpAddr::V4(address)) => address.to_string(),
        Err(_) => client_ip.to_owned(),
    };
    keys.mac("engagement/share-guest", network.as_bytes())[..16].iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The share target of a request; `SHARE_TARGET_INVALID` when unset or unknown.
pub(crate) fn requested_target(value: buffa::EnumValue<ShareTarget>) -> AppResult<ShareTarget> {
    match value.as_known() {
        Some(target) if target != ShareTarget::SHARE_TARGET_UNSPECIFIED => Ok(target),
        _ => Err(AppError::invalid("SHARE_TARGET_INVALID", "say how it was shared")),
    }
}

impl Engagement {
    /// Records that a subject was shared (`EngagementService.RecordShare`)
    /// and returns its new share count. `client_ip` rate-limits guests
    /// (through [`guest_key`], never stored as it is). The same user (or
    /// guest address) sharing the same subject the same way
    /// again within [`REPEAT_WINDOW`] is not counted again: one account
    /// cannot inflate a public number.
    pub(crate) async fn record_share(
        &self,
        user_id: Option<Uuid>,
        client_ip: &str,
        subject: &Subject,
        target: ShareTarget,
    ) -> AppResult<i64> {
        let cache = &self.core.cache;
        let sharer = match user_id {
            Some(user_id) => user_id.to_string(),
            None => format!("guest:{}", guest_key(&self.core, client_ip)),
        };
        match user_id {
            Some(user_id) => {
                cache
                    .rate_limit(&format!("engagement:share:{user_id}"), SHARES_PER_MINUTE, Duration::from_secs(60))
                    .await?
            }
            None => {
                cache
                    .rate_limit(&format!("engagement:share:{sharer}"), GUEST_SHARES_PER_MINUTE, Duration::from_secs(60))
                    .await?
            }
        }
        let key = subject.key;
        let repeat_key = format!("engagement:shared:{sharer}:{}:{}:{}", key.kind, key.id, target as i32);
        if !cache.set_nx(&repeat_key, "1", REPEAT_WINDOW).await.unwrap_or(true) {
            return Ok(stats::load(&self.core, &[key]).await?.of(key).share_count);
        }
        let mut tx = self.core.db.begin().await?;
        let mut effects = Effects::default();
        sqlx::query(
            "INSERT INTO share_counts (subject_type, subject_id, target, count) VALUES ($1, $2, $3, 1)
             ON CONFLICT (subject_type, subject_id, target) DO UPDATE SET count = share_counts.count + 1",
        )
        .bind(key.kind)
        .bind(key.id)
        .bind(target as i32)
        .execute(&mut *tx)
        .await?;
        // The statistics row is always the last lock taken.
        stats::add(&mut tx, key, Some(subject.info.owner_id), Counter::Share, 1).await?;
        effects.count(key, Counter::Share, 1);
        let share_count: i64 =
            sqlx::query_scalar("SELECT share_count FROM subject_stats WHERE subject_type = $1 AND subject_id = $2")
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
        let targets = rows
            .into_iter()
            .map(|(target, count)| ShareTargetCount { target: target.into(), count, ..Default::default() })
            .collect();
        Ok((total, targets))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guests_are_keyed_by_a_keyed_hash_of_their_network() {
        let core_keys = tokuly_core::keys::Keys::new("test-secret-test-secret-test-secret");
        let key = |ip: &str| super::guest_key_with(&core_keys, ip);
        // Neither the address nor its network appears in the key.
        assert!(!key("203.0.113.7").contains("203"));
        assert_eq!(key("203.0.113.7").len(), 32);
        assert_ne!(key("203.0.113.7"), key("203.0.113.8"));
        // One IPv6 /64 is one guest; another /64 is another.
        assert_eq!(key("2001:db8:1:2::1"), key("2001:db8:1:2:ffff::9"));
        assert_ne!(key("2001:db8:1:2::1"), key("2001:db8:1:3::1"));
        // An IPv4 address written as IPv6 is the same guest.
        assert_eq!(key("::ffff:203.0.113.7"), key("203.0.113.7"));
        // Another secret, other keys.
        let other = tokuly_core::keys::Keys::new("another-secret-another-secret-123");
        assert_ne!(super::guest_key_with(&other, "203.0.113.7"), key("203.0.113.7"));
    }

    #[test]
    fn a_share_needs_a_known_target() {
        assert_eq!(requested_target(ShareTarget::SHARE_TARGET_CHAT.into()).unwrap(), ShareTarget::SHARE_TARGET_CHAT);
        assert_eq!(
            requested_target(ShareTarget::SHARE_TARGET_UNSPECIFIED.into()).unwrap_err().reason,
            "SHARE_TARGET_INVALID"
        );
        assert_eq!(requested_target(buffa::EnumValue::from(99)).unwrap_err().reason, "SHARE_TARGET_INVALID");
    }
}
