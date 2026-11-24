//! Background work of the engagement domain: reacting to other domains'
//! events, and forgetting the comment texts kept for reports once they are
//! old enough. Counters are exact by construction and need no timer.
use std::time::Duration;

use tokuly_core::{
    events::{self, ConsumeOptions, Event},
    prelude::*,
};
use tokuly_identity::maintenance::UserRef;
use tokuly_social::events::{BlockChanged, FriendPrefsChanged, FriendRemoved};

use crate::{
    Engagement,
    events::{
        ASSET_DELETED, AssetDeleted, CONTENT_REMOVED, FRIEND_PREFS_CHANGED, FRIEND_REMOVED, MUTE_CHANGED, MuteChanged,
        REPORT_RESOLVED, USER_BLOCKED, USER_DELETED,
    },
};

/// How often removed comment texts past their retention are forgotten.
const REMOVED_TEXT_PURGE_INTERVAL: Duration = Duration::from_secs(3600);

impl Engagement {
    /// Starts the engagement workers (worker processes only): the durable
    /// consumer `engagement` of `identity.user.deleted`,
    /// `social.user.blocked`, `social.friend.removed`,
    /// `social.friend.prefs_changed`, `notify.mute.changed`,
    /// `media.asset.deleted`, `admin.content.removed` and
    /// `admin.report.resolved`, and the hourly
    /// purge of removed comment texts.
    pub fn spawn_workers(&self) {
        let purging = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(REMOVED_TEXT_PURGE_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = purging.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        if let Err(error) = purging.purge_removed_comment_texts().await {
                            tracing::warn!(%error, "removed comment text purge failed");
                        }
                    }
                }
            }
        });
        let engagement = self.clone();
        tokio::spawn(async move {
            let handler = engagement.clone();
            events::consume(
                engagement.core.bus.clone(),
                "engagement",
                &[
                    USER_DELETED,
                    USER_BLOCKED,
                    FRIEND_REMOVED,
                    FRIEND_PREFS_CHANGED,
                    MUTE_CHANGED,
                    ASSET_DELETED,
                    CONTENT_REMOVED,
                    REPORT_RESOLVED,
                ],
                // Erasing an account or purging a busy subject takes a while.
                ConsumeOptions { ack_wait: Duration::from_secs(120), ..Default::default() },
                engagement.core.shutdown.clone(),
                move |event| {
                    let engagement = handler.clone();
                    async move { engagement.handle_event(event).await }
                },
            )
            .await;
        });
    }

    /// Dispatches a consumed event (see [`cleanup`](crate::cleanup) for the
    /// reactions). Public so tests can deliver events without a running relay.
    pub async fn handle_event(&self, event: Event) -> AppResult<()> {
        match event.topic.as_str() {
            USER_DELETED => self.on_user_deleted(event.data::<UserRef>()?.user_id).await,
            USER_BLOCKED => {
                let block: BlockChanged = event.data()?;
                self.on_user_blocked(block.user_id, block.blocked_id).await
            }
            FRIEND_REMOVED => {
                let removed: FriendRemoved = event.data()?;
                self.on_friend_removed(removed.user_id, removed.friend_id).await;
                Ok(())
            }
            FRIEND_PREFS_CHANGED => {
                let changed: FriendPrefsChanged = event.data()?;
                self.on_mute_changed(changed.user_id, changed.friend_id, changed.muted, event.occurred_at).await
            }
            MUTE_CHANGED => {
                let changed: MuteChanged = event.data()?;
                self.on_mute_changed(changed.user_id, changed.muted_user_id, changed.muted, event.occurred_at).await
            }
            ASSET_DELETED => self.on_asset_deleted(event.data::<AssetDeleted>()?.asset_id).await,
            CONTENT_REMOVED => self.on_content_removed(event.data()?).await,
            REPORT_RESOLVED => self.on_report_resolved(event.data()?).await,
            _ => Ok(()),
        }
    }
}
