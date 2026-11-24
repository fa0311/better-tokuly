//! Notices in the conversations of a bot, posted by the chat domain.
//!
//! The chat domain is not one this crate may depend on; the server bridges
//! this trait to `tokuly_chat::Chat::post_notice`
//! (`crates/server/src/bridges.rs`); tests use a stand-in.
use async_trait::async_trait;
use tokuly_core::{events::Event, outbound::Localized, prelude::*};

use crate::{Dev, events::BotTransferEvent};

#[async_trait]
pub trait ConversationNotices: Send + Sync + 'static {
    /// Has `notice` posted into every direct conversation of `account`, as
    /// a notice from the server (no sender): queued, posted in the
    /// background. Each one is posted once per `key`, so a repeated call is
    /// harmless.
    async fn post_to_direct_conversations(&self, account: Uuid, key: &str, notice: Localized) -> AppResult<()>;
}

/// What a bot's conversations are told when it changes hands.
pub fn bot_has_new_owner() -> Localized {
    Localized::new("このボットの持ち主が変わったよ", "This bot has a new owner")
}

impl Dev {
    /// `dev.bot.transferred`: the people who talk with the bot are told that
    /// someone else runs it now (who can read what they sent it). Once per
    /// event, however often it is delivered; a hand-over that arrives late
    /// still happened, so it is told all the same.
    pub(crate) async fn on_bot_transferred(&self, event: &Event) -> AppResult<()> {
        let Some(notices) = self.notices.read().clone() else { return Ok(()) };
        let transfer: BotTransferEvent = event.data()?;
        let key = format!("dev.bot.transferred:{}", event.id);
        notices.post_to_direct_conversations(transfer.bot_id, &key, bot_has_new_owner()).await
    }
}
