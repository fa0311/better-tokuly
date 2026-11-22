//! Glue between domains that must not depend on each other.
//!
//! A domain that needs something from a domain above it in the dependency
//! graph defines a small trait for it; the bridge here implements that trait
//! with the other domain's API. The server is the only crate that sees both
//! sides, so the domains stay independent.
use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use tokuly_core::prelude::*;
use tokuly_engagement::ChannelDirectory;
use tokuly_proto::common::{ChannelSummary, StampRef};
use tokuly_stamp::SendContext;
use tokuly_video::PartyStamps;

use crate::app::Domains;

/// Connects the domains to each other. Called once, after all of them exist.
pub fn connect(core: &Core, domains: &Domains) {
    domains.video.set_party_stamps(Arc::new(StampsInParties { core: core.clone() }));
    domains.engagement.set_channel_directory(Arc::new(ChannelsForComments { core: core.clone() }));
}

/// Stamps in watch-party chat: the video domain asks, the stamp domain decides.
struct StampsInParties {
    core: Core,
}

#[async_trait]
impl PartyStamps for StampsInParties {
    async fn send_stamp(&self, user_id: Uuid, stamp_id: Uuid) -> AppResult<StampRef> {
        let stamp = tokuly_stamp::ensure_usable(&self.core, user_id, stamp_id).await?;
        // Party chat is live chat as far as usage statistics go.
        tokuly_stamp::record_send(&self.core, user_id, stamp_id, SendContext::LiveChat).await?;
        Ok(stamp)
    }
}

/// Commenting as a channel: engagement asks, the video domain owns channels.
struct ChannelsForComments {
    core: Core,
}

#[async_trait]
impl ChannelDirectory for ChannelsForComments {
    async fn owned_channel(&self, user_id: Uuid, channel_id: Uuid) -> AppResult<Option<ChannelSummary>> {
        let owned = tokuly_video::channels::owned_by(&self.core, user_id).await?;
        if !owned.iter().any(|channel| channel.id == channel_id) {
            return Ok(None);
        }
        Ok(self.summaries(&[channel_id]).await?.remove(&channel_id))
    }

    async fn summaries(&self, ids: &[Uuid]) -> AppResult<HashMap<Uuid, ChannelSummary>> {
        tokuly_video::channels::summaries(&self.core, ids).await
    }
}
