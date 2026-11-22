//! Glue between domains that must not depend on each other.
//!
//! A domain that needs something from a domain above it in the dependency
//! graph defines a small trait for it; the bridge here implements that trait
//! with the other domain's API. The server is the only crate that sees both
//! sides, so the domains stay independent.
use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use sqlx::PgConnection;
use tokuly_admin::HiddenOwners;
use tokuly_community::Community;
use tokuly_core::prelude::*;
use tokuly_dev::ImageStore;
use tokuly_engagement::ChannelDirectory;
use tokuly_media::assets;
use tokuly_proto::{
    common::{ChannelSummary, StampRef, SubjectType},
    media::AssetPurpose,
};
use tokuly_quest::DirectChats;
use tokuly_stamp::SendContext;
use tokuly_video::PartyStamps;

use crate::app::Domains;

/// Connects the domains to each other. Called once, after all of them exist.
pub fn connect(core: &Core, domains: &Domains) {
    domains.video.set_party_stamps(Arc::new(StampsInParties { core: core.clone() }));
    domains.engagement.set_channel_directory(Arc::new(ChannelsForComments { core: core.clone() }));
    domains.quest.set_direct_chats(Arc::new(DirectChatsForStreaks { core: core.clone() }));
    domains.dev.set_images(Arc::new(ImagesForDevelopers { core: core.clone() }));
    domains.admin.set_hidden_owners(Arc::new(AnonymousAuthors { community: domains.community.clone() }));
    // Not connected: `Shorts::set_clip_directory`. Clips belong to the live
    // domain, which this binary does not serve yet; until it does, a short
    // can only be made from the caller's own upload.
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

/// Friend streaks: quest hears about a message with its conversation; chat
/// knows who the two people of a direct conversation are.
struct DirectChatsForStreaks {
    core: Core,
}

#[async_trait]
impl DirectChats for DirectChatsForStreaks {
    async fn members(&self, conversation_id: Uuid) -> AppResult<Option<(Uuid, Uuid)>> {
        tokuly_chat::direct_members(&self.core, conversation_id).await
    }
}

/// Application icons and bot avatars: the developer platform names an
/// upload, the media domain owns it.
struct ImagesForDevelopers {
    core: Core,
}

/// What an icon or a bot's avatar may have been uploaded as.
const DEVELOPER_IMAGE_PURPOSES: &[AssetPurpose] =
    &[AssetPurpose::ASSET_PURPOSE_AVATAR, AssetPurpose::ASSET_PURPOSE_CHANNEL_ICON];

#[async_trait]
impl ImageStore for ImagesForDevelopers {
    async fn image(&self, uploader: Uuid, asset_id: Uuid) -> AppResult<serde_json::Value> {
        assets::require_image_json(&self.core, asset_id, uploader, DEVELOPER_IMAGE_PURPOSES).await
    }

    async fn hold(&self, tx: &mut PgConnection, asset_id: Uuid, holder: &str) -> AppResult<()> {
        // A slot holds one image. Both steps run on the caller's
        // transaction, so setting the image the slot already has ends as it began.
        assets::detach_holder(&mut *tx, holder).await?;
        assets::attach(&mut *tx, asset_id, holder).await?;
        Ok(())
    }

    async fn release(&self, tx: &mut PgConnection, holder: &str) -> AppResult<()> {
        assets::detach_holder(&mut *tx, holder).await?;
        Ok(())
    }
}

/// Reports about posts in anonymous channels arrive without an owner: the
/// admin domain asks, the community domain knows who wrote them. Only the
/// moderation queue gets the answer.
struct AnonymousAuthors {
    community: Community,
}

#[async_trait]
impl HiddenOwners for AnonymousAuthors {
    async fn owner_for_moderation(&self, subject_type: SubjectType, subject_id: Uuid) -> AppResult<Option<Uuid>> {
        self.community.author_for_moderation(subject_type, subject_id).await
    }
}
