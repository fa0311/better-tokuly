//! Background work of the stamp domain: reacting to other domains' events
//! and the periodic upkeep of rankings and the usage log.
use std::time::Duration;

use tokuly_core::{
    events::{self, ConsumeOptions, Event},
    prelude::*,
};
use tokuly_identity::maintenance::UserRef;

use crate::{
    Stamp,
    events::{ASSET_DELETED, AssetDeleted, CONTENT_REMOVED, USER_DELETED},
    stats,
};

/// How often the usage log is pruned.
const PRUNE_INTERVAL: Duration = Duration::from_secs(3600);
/// How often purchases nobody finished are settled.
const SETTLE_INTERVAL: Duration = Duration::from_secs(300);

impl Stamp {
    /// Starts the stamp workers (worker processes only):
    /// - the consumer of `identity.user.deleted`, `media.asset.deleted` and
    ///   `admin.content.removed`;
    /// - the ranking refresh, every `ranking_interval_seconds`;
    /// - the hourly pruning of the usage log;
    /// - every five minutes, the settling of unfinished purchases;
    /// - once at startup, the seeding of the official packs.
    pub fn spawn_workers(&self) {
        let stamp = self.clone();
        tokio::spawn(async move {
            let handler = stamp.clone();
            events::consume(
                stamp.core.bus.clone(),
                "stamp",
                &[USER_DELETED, ASSET_DELETED, CONTENT_REMOVED],
                ConsumeOptions::default(),
                stamp.core.shutdown.clone(),
                move |event| {
                    let stamp = handler.clone();
                    async move { stamp.handle_event(event).await }
                },
            )
            .await;
        });

        let stamp = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(stamp.cfg.ranking_interval_seconds.max(10)));
            loop {
                tokio::select! {
                    _ = stamp.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        if let Err(e) = stats::refresh_rankings(&stamp.core).await {
                            tracing::error!(error = %e, "stamp ranking refresh failed");
                        }
                    }
                }
            }
        });

        let stamp = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(PRUNE_INTERVAL);
            loop {
                tokio::select! {
                    _ = stamp.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        match stats::prune(&stamp.core, stamp.cfg.usage_retention_days).await {
                            Ok(0) => {}
                            Ok(rows) => tracing::info!(rows, "pruned the stamp usage log"),
                            Err(e) => tracing::error!(error = %e, "stamp usage log pruning failed"),
                        }
                    }
                }
            }
        });

        let stamp = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(SETTLE_INTERVAL);
            loop {
                tokio::select! {
                    _ = stamp.core.shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        if let Err(e) = stamp.settle_unfinished_purchases().await {
                            tracing::error!(error = %e, "settling unfinished stamp purchases failed");
                        }
                    }
                }
            }
        });

        if self.cfg.seed_official {
            let stamp = self.clone();
            tokio::spawn(async move {
                if let Err(e) = stamp.seed_official_packs().await {
                    tracing::error!(error = %e, "seeding the official stamp packs failed");
                }
            });
        }
    }

    /// Dispatches a consumed event (see [`cleanup`](crate::cleanup) for the
    /// reactions). Public so tests can deliver events without a running relay.
    pub async fn handle_event(&self, event: Event) -> AppResult<()> {
        match event.topic.as_str() {
            USER_DELETED => self.on_user_deleted(event.data::<UserRef>()?.user_id).await,
            ASSET_DELETED => self.on_asset_deleted(event.data::<AssetDeleted>()?.asset_id).await,
            CONTENT_REMOVED => self.on_content_removed(event.data()?).await,
            _ => Ok(()),
        }
    }
}
