//! Reactions to what happens in other domains: accounts are deleted, users
//! block and befriend each other, uploads finish processing or disappear,
//! moderation takes a message down. Every reaction is idempotent (events
//! are delivered at least once).
use tokuly_core::{events::Event, prelude::*};
use tokuly_identity::maintenance::{BirthdateGiven, UserRef};
use tokuly_media::assets;
use tokuly_proto::{chat as pb, common::SubjectType, media::Attachment};
use tokuly_social::{
    events::{BlockChanged, FriendAccepted},
    graph,
};

use crate::{
    Chat, access,
    conversations::delete_conversation,
    events::{
        ASSET_DELETED, ASSET_FAILED, ASSET_READY, AssetRef, BIRTHDATE_GIVEN, CONTENT_REMOVED, ContentRemoved,
        FRIEND_ACCEPTED, USER_BLOCKED, USER_CREATED, USER_DELETED, USER_UNBLOCKED,
    },
    model::{MSG, MessageRow, kind, msg},
};

impl Chat {
    /// Reacts to one event from another domain. Public so tests can
    /// deliver events directly (no relay runs in tests).
    pub async fn handle_event(&self, event: &Event) -> AppResult<()> {
        match event.topic.as_str() {
            // The age defaults (consent, Tokuly Bot) are fixed when the account is created.
            USER_CREATED => self.store_age_defaults(&[event.data::<UserRef>()?.user_id]).await.map(drop),
            // ...or when a minor gives the birthdate later.
            BIRTHDATE_GIVEN => match event.data::<BirthdateGiven>()? {
                BirthdateGiven { user_id, minor: true } => self.apply_minor_defaults(user_id).await,
                BirthdateGiven { minor: false, .. } => Ok(()),
            },
            USER_DELETED => self.on_user_deleted(event.data::<UserRef>()?.user_id).await,
            USER_BLOCKED | USER_UNBLOCKED => {
                let change: BlockChanged = event.data()?;
                self.on_block_changed(change.user_id, change.blocked_id).await
            }
            FRIEND_ACCEPTED => {
                let friends: FriendAccepted = event.data()?;
                self.on_friends(friends.user_id, friends.friend_id).await
            }
            ASSET_READY | ASSET_FAILED => self.on_asset_processed(event.data::<AssetRef>()?.asset_id).await,
            ASSET_DELETED => self.on_asset_deleted(event.data::<AssetRef>()?.asset_id).await,
            CONTENT_REMOVED => self.on_content_removed(event.data()?).await,
            _ => Ok(()),
        }
    }

    /// Brings the read-only flag of a direct conversation in line with the
    /// facts: nobody can write when one of the two blocked the other or one
    /// side's membership is gone. Becoming read-only also ends a live call
    /// and closes an open POV request.
    pub(crate) async fn refresh_direct_state(&self, conversation_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        let Some((lo, hi)) = access::find_conversation(core, conversation_id).await?.and_then(|conv| conv.pair())
        else {
            return Ok(());
        };
        let blocked = graph::is_blocked_either(core, lo, hi).await?;
        let mut tx = core.db.begin().await?;
        // Deleted meanwhile: both accounts were erased.
        let Some(conv) = access::lock_conversation_if_present(&mut tx, conversation_id).await? else { return Ok(()) };
        let members: i64 = sqlx::query_scalar("SELECT count(*) FROM chat_members WHERE conversation_id = $1")
            .bind(conversation_id)
            .fetch_one(&mut *tx)
            .await?;
        let read_only = members < 2 || blocked;
        if read_only == conv.read_only {
            return Ok(());
        }
        sqlx::query("UPDATE chat_conversations SET read_only = $2 WHERE id = $1")
            .bind(conversation_id)
            .bind(read_only)
            .execute(&mut *tx)
            .await?;
        let (call, request) = if read_only {
            (
                self.fail_live_call(&mut tx, conversation_id).await?,
                self.cancel_open_pov(&mut tx, conversation_id).await?,
            )
        } else {
            (None, None)
        };
        tx.commit().await?;
        self.push_conversation_shared(conversation_id).await;
        if let Some(call) = &call {
            self.after_call_change(call).await;
        }
        if let Some(request) = &request {
            self.after_pov_closed(request).await;
        }
        Ok(())
    }

    /// `social.user.blocked` / `social.user.unblocked`: the direct
    /// conversation of the two becomes read-only for both, or writable again.
    pub(crate) async fn on_block_changed(&self, a: Uuid, b: Uuid) -> AppResult<()> {
        let conversation: Option<Uuid> = self.direct_conversation_id(a, b).await?;
        match conversation {
            Some(conversation_id) => self.refresh_direct_state(conversation_id).await,
            None => Ok(()),
        }
    }

    async fn direct_conversation_id(&self, a: Uuid, b: Uuid) -> AppResult<Option<Uuid>> {
        let (lo, hi) = crate::conversations::pair_of(a, b);
        Ok(sqlx::query_scalar("SELECT id FROM chat_conversations WHERE kind = $1 AND user_lo = $2 AND user_hi = $3")
            .bind(kind::DIRECT)
            .bind(lo)
            .bind(hi)
            .fetch_optional(&self.core.db)
            .await?)
    }

    /// `social.friend.accepted`: friends need no message request.
    pub(crate) async fn on_friends(&self, a: Uuid, b: Uuid) -> AppResult<()> {
        let Some(conversation_id) = self.direct_conversation_id(a, b).await? else { return Ok(()) };
        for (user_id, other) in [(a, b), (b, a)] {
            if self.accept_request_of(conversation_id, user_id).await? {
                // The other side's allowance is lifted.
                self.push_conversation_to(conversation_id, other).await;
            }
        }
        Ok(())
    }

    /// `identity.user.deleted`: the user's memberships, reactions, call
    /// participations, AI settings and keys, and co-presence go. Their messages stay and show a deleted user; direct
    /// conversations become read-only for the other side, and go once that
    /// side was erased too; groups lose a member (and get a new owner when
    /// they owned one).
    pub(crate) async fn on_user_deleted(&self, user_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        let memberships: Vec<(Uuid, i16)> = sqlx::query_as(
            "SELECT m.conversation_id, c.kind FROM chat_members m JOIN chat_conversations c ON c.id = m.conversation_id
             WHERE m.user_id = $1 ORDER BY m.conversation_id",
        )
        .bind(user_id)
        .fetch_all(&core.db)
        .await?;
        for (conversation_id, conversation_kind) in memberships {
            if conversation_kind == kind::GROUP {
                let mut tx = core.db.begin().await?;
                let Some(mut conv) = access::lock_conversation_if_present(&mut tx, conversation_id).await? else {
                    continue;
                };
                let Some(member) = access::member_in_tx(&mut tx, conversation_id, user_id).await? else { continue };
                let departure = self.remove_member_tx(&mut tx, &mut conv, &member, None).await?;
                tx.commit().await?;
                self.after_departure(&departure, pb::ConversationRemovalReason::CONVERSATION_REMOVAL_REASON_LEFT).await;
            } else {
                let mut tx = core.db.begin().await?;
                // Gone already: the other side was erased meanwhile and took it along.
                let Some(conv) = access::lock_conversation_if_present(&mut tx, conversation_id).await? else {
                    continue;
                };
                let removed = sqlx::query("DELETE FROM chat_members WHERE conversation_id = $1 AND user_id = $2")
                    .bind(conversation_id)
                    .bind(user_id)
                    .execute(&mut *tx)
                    .await?;
                let left = if removed.rows_affected() > 0 {
                    sqlx::query_scalar(
                        "UPDATE chat_conversations SET member_count = member_count - 1 WHERE id = $1 RETURNING member_count",
                    )
                    .bind(conversation_id)
                    .fetch_one(&mut *tx)
                    .await?
                } else {
                    conv.member_count
                };
                // Nobody is left to read it: it goes like an empty group.
                if left <= 0 {
                    delete_conversation(&mut tx, conversation_id).await?;
                }
                tx.commit().await?;
                self.forget_presence(conversation_id, user_id).await;
                if left > 0 {
                    self.refresh_direct_state(conversation_id).await?;
                }
            }
        }
        sqlx::query("DELETE FROM chat_reactions WHERE user_id = $1").bind(user_id).execute(&core.db).await?;
        // Where they were in calls (after the loop above took them out of
        // live ones); the calls themselves stay for the others.
        sqlx::query("DELETE FROM chat_call_participants WHERE user_id = $1").bind(user_id).execute(&core.db).await?;
        // Their AI keys and settings; their messages then follow the
        // default for an unknown age: not offered to the AI.
        sqlx::query("DELETE FROM chat_ai_user_keys WHERE user_id = $1").bind(user_id).execute(&core.db).await?;
        sqlx::query("DELETE FROM chat_ai_user_settings WHERE user_id = $1").bind(user_id).execute(&core.db).await?;
        Ok(())
    }

    /// Deletes the direct conversations whose two accounts are both erased,
    /// as the second erasure does ([`Chat::on_user_deleted`]). Erasures
    /// handled before it did left them behind with nobody in them; run once
    /// at every worker start, it finds nothing after the first. Returns how
    /// many it deleted.
    pub async fn sweep_erased_directs(&self) -> AppResult<usize> {
        let core = &self.core;
        let mut deleted = 0;
        let mut after = Uuid::nil();
        loop {
            let batch: Vec<Uuid> = sqlx::query_scalar(
                "SELECT c.id FROM chat_conversations c
                 JOIN users lo ON lo.id = c.user_lo JOIN users hi ON hi.id = c.user_hi
                 WHERE c.kind = $1 AND c.id > $2 AND lo.deleted_at IS NOT NULL AND hi.deleted_at IS NOT NULL
                 ORDER BY c.id LIMIT 100",
            )
            .bind(kind::DIRECT)
            .bind(after)
            .fetch_all(&core.db)
            .await?;
            for &conversation_id in &batch {
                // One transaction each: a long history makes a big one.
                let mut tx = core.db.begin().await?;
                // Gone already: an erasure or another worker took it.
                if access::lock_conversation_if_present(&mut tx, conversation_id).await?.is_some() {
                    delete_conversation(&mut tx, conversation_id).await?;
                    tx.commit().await?;
                    deleted += 1;
                }
            }
            match batch.last() {
                Some(&last) if batch.len() == 100 => after = last,
                _ => return Ok(deleted),
            }
        }
    }

    /// Messages showing an upload.
    async fn messages_of_asset(&self, asset_id: Uuid) -> AppResult<Vec<Uuid>> {
        Ok(sqlx::query_scalar("SELECT message_id FROM chat_message_assets WHERE asset_id = $1 ORDER BY message_id")
            .bind(asset_id)
            .fetch_all(&self.core.db)
            .await?)
    }

    /// Rewrites the stored attachments of one message under a lock on it.
    /// `change` returns `false` to leave the message alone.
    async fn rewrite_attachments(
        &self,
        message_id: Uuid,
        change: impl FnOnce(&mut Vec<Attachment>) -> bool,
    ) -> AppResult<Option<MessageRow>> {
        let mut tx = self.core.db.begin().await?;
        let row: Option<MessageRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {MSG} FROM chat_messages x WHERE x.id = $1 FOR UPDATE"
        )))
        .bind(message_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row.filter(|row| !row.is_deleted()) else { return Ok(None) };
        let mut attachments = row.attachments();
        if !change(&mut attachments) {
            return Ok(None);
        }
        let json = serde_json::to_value(&attachments)
            .map_err(|e| AppError::internal(anyhow::anyhow!("attachments encode: {e}")))?;
        let updated: MessageRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE chat_messages x SET attachments = $2 WHERE x.id = $1 RETURNING {MSG}"
        )))
        .bind(message_id)
        .bind(json)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some(updated))
    }

    /// `media.asset.ready` / `media.asset.failed`: a video sent while it was
    /// still transcoding gets its final shape in every message showing it.
    pub(crate) async fn on_asset_processed(&self, asset_id: Uuid) -> AppResult<()> {
        let message_ids = self.messages_of_asset(asset_id).await?;
        if message_ids.is_empty() {
            return Ok(());
        }
        let Some(asset) = assets::find(&self.core, asset_id).await? else { return Ok(()) };
        let fresh = asset.to_attachment();
        sqlx::query("UPDATE chat_message_assets SET kind = $2 WHERE asset_id = $1")
            .bind(asset_id)
            .bind(asset.kind as i16)
            .execute(&self.core.db)
            .await?;
        for message_id in message_ids {
            let updated = self
                .rewrite_attachments(message_id, |attachments| {
                    let mut changed = false;
                    for attachment in attachments.iter_mut().filter(|attachment| attachment.asset_id == fresh.asset_id)
                    {
                        changed |= *attachment != fresh;
                        *attachment = fresh.clone();
                    }
                    changed
                })
                .await?;
            if let Some(updated) = updated {
                self.push_message_updated(&updated, pb::MessageUpdateReason::MESSAGE_UPDATE_REASON_ATTACHMENT).await;
            }
        }
        Ok(())
    }

    /// `media.asset.deleted`: an upload was removed regardless of who held
    /// it (a takedown). It leaves every message showing it — a media message
    /// left with nothing at all becomes a tombstone — and a group using it
    /// as icon loses the icon.
    pub(crate) async fn on_asset_deleted(&self, asset_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        let groups: Vec<Uuid> = sqlx::query_scalar(
            "UPDATE chat_conversations SET icon = NULL, icon_asset_id = NULL WHERE icon_asset_id = $1 RETURNING id",
        )
        .bind(asset_id)
        .fetch_all(&core.db)
        .await?;
        for conversation_id in groups {
            self.push_conversation_shared(conversation_id).await;
        }
        let gone = asset_id.to_string();
        for message_id in self.messages_of_asset(asset_id).await? {
            let updated = self
                .rewrite_attachments(message_id, |attachments| {
                    let before = attachments.len();
                    attachments.retain(|attachment| attachment.asset_id != gone);
                    attachments.len() != before
                })
                .await?;
            sqlx::query("DELETE FROM chat_message_assets WHERE message_id = $1 AND asset_id = $2")
                .bind(message_id)
                .bind(asset_id)
                .execute(&core.db)
                .await?;
            let Some(updated) = updated else { continue };
            let empty = updated.kind == msg::MEDIA && updated.attachments().is_empty() && updated.text.is_empty();
            if empty {
                self.tombstone(message_id, None, false).await?;
            } else {
                self.push_message_updated(&updated, pb::MessageUpdateReason::MESSAGE_UPDATE_REASON_ATTACHMENT).await;
            }
        }
        Ok(())
    }

    /// `admin.content.removed`: a moderator took a chat message down.
    pub(crate) async fn on_content_removed(&self, removed: ContentRemoved) -> AppResult<()> {
        if removed.subject_type != SubjectType::SUBJECT_TYPE_CHAT_MESSAGE as i32 {
            return Ok(());
        }
        self.tombstone(removed.subject_id, removed.moderator_id, true).await?;
        Ok(())
    }
}
