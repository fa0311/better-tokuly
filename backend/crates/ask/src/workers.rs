//! Background work: the event consumer and the maintenance timer.
//!
//! One durable consumer, `ask`, handles both this domain's own delivery
//! events (see `delivery.rs`) and what other domains announce (see
//! `cleanup.rs` and `friends.rs`).
use std::time::Duration;

use tokuly_core::{
    events::{self, ConsumeOptions, Event},
    prelude::*,
};
use tokuly_identity::maintenance::UserRef;

use crate::{
    Ask,
    events::{
        ANSWER_PUBLISHED, ASSET_DELETED, AssetDeleted, CONSUMED, CONTENT_REMOVED, FLAME_STORED, FRIEND_REMOVED, PROMPT_OPENED, QUESTION_STORED, RESPONSE_STORED, Stored, USER_BANNED,
        USER_BLOCKED, USER_DELETED, USER_UNBANNED,
    },
};

/// Durable name of this domain's event consumer.
const CONSUMER: &str = "ask";
/// Pause between purges.
const PURGE_INTERVAL: Duration = Duration::from_secs(600);

impl Ask {
    /// Starts the workers: the event consumer and the purge timer. Also
    /// copies the built-in poll questions into the database.
    pub fn spawn_workers(&self) {
        let ask = self.clone();
        tokio::spawn(async move {
            if let Err(e) = ask.ensure_bank().await {
                tracing::error!(error = %e, "loading the built-in poll questions failed; the first poll will try again");
            }
        });

        let ask = self.clone();
        tokio::spawn(async move {
            let handler = ask.clone();
            events::consume(
                ask.core.bus.clone(),
                CONSUMER,
                CONSUMED,
                ConsumeOptions::default(),
                ask.core.shutdown.clone(),
                move |event| {
                    let ask = handler.clone();
                    async move { ask.handle_event(event).await }
                },
            )
            .await;
        });

        let ask = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(PURGE_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = ask.core.shutdown.cancelled() => break,
                    _ = tick.tick() => match ask.purge().await {
                        Ok(0) => {}
                        Ok(rows) => tracing::info!(rows, "purged expired ask data"),
                        Err(e) => tracing::error!(error = %e, "purging expired ask data failed"),
                    },
                }
            }
        });
    }

    /// Handles one consumed event. Events are delivered at least once, so
    /// everything behind this is safe to repeat.
    pub async fn handle_event(&self, event: Event) -> AppResult<()> {
        match event.topic.as_str() {
            QUESTION_STORED => self.deliver_question(event.data::<Stored>()?.id).await,
            RESPONSE_STORED => self.deliver_response(event.data::<Stored>()?.id).await,
            FLAME_STORED => self.deliver_flame(event.data::<Stored>()?.id).await,
            ANSWER_PUBLISHED => self.tell_asker(event.data()?).await,
            PROMPT_OPENED => self.tell_friends(event.data()?).await,
            USER_DELETED => self.on_user_deleted(event.data::<UserRef>()?.user_id).await,
            USER_BANNED => self.on_ban_changed(event.data::<UserRef>()?.user_id, true).await,
            USER_UNBANNED => self.on_ban_changed(event.data::<UserRef>()?.user_id, false).await,
            USER_BLOCKED => self.on_user_blocked(event.data()?).await,
            FRIEND_REMOVED => self.on_friend_removed(event.data()?).await,
            CONTENT_REMOVED => self.on_content_removed(event.data()?).await,
            ASSET_DELETED => self.on_asset_deleted(event.data::<AssetDeleted>()?.asset_id).await,
            _ => Ok(()),
        }
    }
}
