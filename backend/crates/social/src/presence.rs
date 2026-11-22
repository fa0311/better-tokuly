//! Presence fan-out: telling a user's friends when they come online or go
//! offline.
//!
//! The realtime gateway only knows connections. It announces transitions on
//! the bus ([`tokuly_core::presence::PRESENCE_SUBJECT`]); this worker knows
//! who the friends are and whether the user shares their online status at
//! all, and pushes a `PresenceEvent` to each friend.
use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures::StreamExt as _;
use tokio::sync::Semaphore;
use tokuly_core::{
    prelude::*,
    presence::{self, PRESENCE_SUBJECT, PresenceChange},
};
use tokuly_identity::settings;
use tokuly_proto::realtime::PresenceEvent;

use crate::{Social, graph};

/// Worker processes share the announcements through this queue group, so
/// each transition is fanned out once.
const QUEUE_GROUP: &str = "social-presence";
/// Transitions fanned out concurrently by one process.
const CONCURRENCY: usize = 16;

impl Social {
    /// Consumes presence announcements until shutdown, resubscribing if the
    /// subscription ends.
    pub(crate) async fn run_presence_fanout(&self) {
        let shutdown = self.core.shutdown.clone();
        let limiter = Arc::new(Semaphore::new(CONCURRENCY));
        while !shutdown.is_cancelled() {
            let mut announcements = match self.core.bus.queue_subscribe(PRESENCE_SUBJECT, QUEUE_GROUP).await {
                Ok(subscription) => subscription,
                Err(e) => {
                    tracing::error!(error = %e, "presence subscription failed; retrying");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
            };
            tracing::info!("presence fan-out started");
            loop {
                let message = tokio::select! {
                    _ = shutdown.cancelled() => return,
                    message = announcements.next() => message,
                };
                let Some(message) = message else { break };
                let Ok(change) = serde_json::from_slice::<PresenceChange>(&message.payload) else {
                    tracing::warn!("undecodable presence announcement dropped");
                    continue;
                };
                let permit = limiter.clone().acquire_owned().await.expect("semaphore is never closed");
                let social = self.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    if let Err(e) = social.fan_out_presence(&change).await {
                        tracing::warn!(user_id = %change.user_id, error = %e, "presence fan-out failed");
                    }
                });
            }
        }
    }

    /// Tells the friends of a user who shares their online status that the
    /// user came online or went offline. Public so tests can deliver a
    /// transition directly.
    pub async fn fan_out_presence(&self, change: &PresenceChange) -> AppResult<()> {
        let core = &self.core;
        if !settings::privacy_of(core, change.user_id).await?.share_online_status {
            return Ok(());
        }
        self.push_presence(change.user_id, change.online, Some(change.at)).await
    }

    /// Brings friends' view of a user's online state in line with a changed
    /// `share_online_status` setting: hiding makes the user look offline at
    /// once, sharing again reveals them if they are online.
    pub(crate) async fn refresh_presence(&self, user_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        let sharing = settings::privacy_of(core, user_id).await?.share_online_status;
        if !sharing {
            // No timestamp: when they were last seen is hidden as well.
            return self.push_presence(user_id, false, None).await;
        }
        if presence::is_online(&core.cache, user_id).await? {
            self.push_presence(user_id, true, Some(Utc::now())).await?;
        }
        Ok(())
    }

    async fn push_presence(&self, user_id: Uuid, online: bool, at: Option<DateTime<Utc>>) -> AppResult<()> {
        let friends = graph::friend_ids(&self.core, user_id).await?;
        let event =
            PresenceEvent { user_id: user_id.to_string(), online, last_seen_at: opt_ts(at), ..Default::default() };
        self.core.rt.presence_to_users(friends, event).await
    }
}
