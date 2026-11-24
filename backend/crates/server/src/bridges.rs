//! Glue between domains that must not depend on each other.
//!
//! A domain that needs something from a domain above it in the dependency
//! graph defines a small trait for it; the bridge here implements that trait
//! with the other domain's API. The server is the only crate that sees both
//! sides, so the domains stay independent.
use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use sqlx::PgConnection;
use tokuly_admin::{HiddenOwners, ReportEvidence, ReportedSubjects};
use tokuly_community::Community;
use tokuly_core::{outbound::Localized, prelude::*, subject::SubjectInfo};
use tokuly_chat::Chat;
use tokuly_dev::{ConversationNotices, ImageStore};
use tokuly_engagement::{ChannelDirectory, Engagement};
use tokuly_media::assets;
use tokuly_proto::{
    common::{ChannelSummary, StampRef, SubjectType},
    media::AssetPurpose,
};
use tokuly_quest::DirectChats;
use tokuly_shorts::{ClipDirectory, ClipMedia};
use tokuly_stamp::SendContext;
use tokuly_video::{PartyStamps, access::Viewer};

use crate::app::Domains;

/// Connects the domains to each other. Called once, after all of them exist.
pub fn connect(core: &Core, domains: &Domains) {
    domains.video.set_party_stamps(Arc::new(StampsInParties { core: core.clone() }));
    domains.engagement.set_channel_directory(Arc::new(ChannelsForComments { core: core.clone() }));
    domains.quest.set_direct_chats(Arc::new(DirectChatsForStreaks { core: core.clone() }));
    domains.dev.set_images(Arc::new(ImagesForDevelopers { core: core.clone() }));
    domains.dev.set_conversation_notices(Arc::new(NoticesForBots { core: core.clone(), chat: domains.chat.clone() }));
    domains.admin.set_hidden_owners(Arc::new(AnonymousAuthors { community: domains.community.clone() }));
    domains.admin.set_report_evidence(Arc::new(EvidenceOfReports));
    domains.admin.set_reported_subjects(Arc::new(ReportedForModerators { engagement: domains.engagement.clone() }));
    domains.shorts.set_clip_directory(Arc::new(ClipsForShorts { core: core.clone() }));
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

/// Notices in a bot's conversations (it changed hands): the developer
/// platform asks, the chat domain posts them.
struct NoticesForBots {
    core: Core,
    chat: Chat,
}

#[async_trait]
impl ConversationNotices for NoticesForBots {
    async fn post_to_direct_conversations(&self, account: Uuid, key: &str, notice: Localized) -> AppResult<()> {
        // Queued in batches (chat's job posts them): the event consumer that
        // asks is not held up by a bot with thousands of conversations.
        let conversations = tokuly_chat::direct_conversations_of(&self.core, account).await?;
        self.chat.queue_notices(&conversations, key, &notice).await
    }
}

/// Reports about posts in anonymous channels arrive without an owner: the
/// admin domain asks, the community domain knows who wrote them, and who
/// else is involved and must therefore not be told. Only the moderation
/// queue gets the answer.
struct AnonymousAuthors {
    community: Community,
}

#[async_trait]
impl HiddenOwners for AnonymousAuthors {
    async fn owner_for_moderation(&self, subject_type: SubjectType, subject_id: Uuid) -> AppResult<Option<Uuid>> {
        self.community.author_for_moderation(subject_type, subject_id).await
    }

    async fn is_party(&self, subject_type: SubjectType, subject_id: Uuid, user_id: Uuid) -> AppResult<bool> {
        self.community.is_party_for_moderation(subject_type, subject_id, user_id).await
    }
}

/// Uploads of reported content: the admin domain holds them for a case, the
/// domain that keeps the content knows which they are, media keeps them.
/// Chat messages are the content whose uploads the sender can take back
/// (unsend); other domains keep theirs until they delete the content.
struct EvidenceOfReports;

#[async_trait]
impl ReportEvidence for EvidenceOfReports {
    async fn hold(&self, conn: &mut PgConnection, subject_type: SubjectType, subject_id: Uuid, holder: &str) -> AppResult<()> {
        if subject_type == SubjectType::SUBJECT_TYPE_CHAT_MESSAGE {
            tokuly_chat::hold_message_media(conn, subject_id, holder).await?;
        }
        Ok(())
    }

    async fn release(&self, conn: &mut PgConnection, holder: &str) -> AppResult<()> {
        assets::detach_holder(conn, holder).await?;
        Ok(())
    }
}

/// Making a short from a clip: shorts asks, the live domain owns clips.
struct ClipsForShorts {
    core: Core,
}

#[async_trait]
impl ClipDirectory for ClipsForShorts {
    async fn clip_of(&self, user_id: Uuid, clip_id: &str) -> AppResult<Option<ClipMedia>> {
        let Some(clip) = tokuly_live::clips::find_ref(&self.core, clip_id).await? else { return Ok(None) };
        let (true, Some(asset_id)) = (clip.is_ready() && clip.creator_id == user_id, clip.asset_id) else {
            return Ok(None);
        };
        // A short has an audience of its own. Whoever cut a clip from someone
        // else's stream or video may post it only while everyone may watch
        // that source: a clip of a friends-only stream must not leave its
        // audience this way.
        if clip.owner_id != user_id {
            let viewer = Viewer { user_id: Some(user_id), ..Default::default() };
            let source = tokuly_live::clips::source_of(&self.core, &viewer, &clip).await?;
            if !source.is_some_and(|source| source.public) {
                return Ok(None);
            }
        }
        Ok(Some(ClipMedia { clip_id: clip.id, asset_id }))
    }
}

/// Reported content in the case view: admin asks, engagement keeps the
/// reports and asks the owning domain as each reporter.
struct ReportedForModerators {
    engagement: Engagement,
}

#[async_trait]
impl ReportedSubjects for ReportedForModerators {
    async fn reported_subject(
        &self,
        staff_id: Uuid,
        subject_type: SubjectType,
        subject_id: Uuid,
    ) -> AppResult<Option<SubjectInfo>> {
        self.engagement.resolve_reported_subject(staff_id, subject_type, subject_id).await
    }
}
