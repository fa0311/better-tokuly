//! Background work of the engagement domain: reacting to other domains'
//! events. There are no timers; counters are exact by construction and
//! nothing here expires.
use tokuly_core::{
    events::{self, ConsumeOptions, Event},
    prelude::*,
};
use tokuly_identity::maintenance::UserRef;
use tokuly_social::events::{BlockChanged, FriendRemoved};

use crate::{
    Engagement,
    events::{
        ASSET_DELETED, AssetDeleted, CONTENT_REMOVED, FRIEND_REMOVED, REPORT_RESOLVED, USER_BLOCKED, USER_DELETED,
    },
};

impl Engagement {
    /// Starts the engagement workers (worker processes only): the durable
    /// consumer `engagement` of `identity.user.deleted`,
    /// `social.user.blocked`, `social.friend.removed`, `media.asset.deleted`,
    /// `admin.content.removed` and `admin.report.resolved`.
    pub fn spawn_workers(&self) {
        let engagement = self.clone();
        tokio::spawn(async move {
            let handler = engagement.clone();
            events::consume(
                engagement.core.bus.clone(),
                "engagement",
                &[USER_DELETED, USER_BLOCKED, FRIEND_REMOVED, ASSET_DELETED, CONTENT_REMOVED, REPORT_RESOLVED],
                // Erasing an account or purging a busy subject takes a while.
                ConsumeOptions { ack_wait: std::time::Duration::from_secs(120), ..Default::default() },
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
            ASSET_DELETED => self.on_asset_deleted(event.data::<AssetDeleted>()?.asset_id).await,
            CONTENT_REMOVED => self.on_content_removed(event.data()?).await,
            REPORT_RESOLVED => self.on_report_resolved(event.data()?).await,
            _ => Ok(()),
        }
    }
}
