//! `LiveChatService`: live chat and its moderation.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use serde::{Deserialize, Serialize};
use tokuly_core::prelude::*;
use tokuly_proto::live::{live_event::Event, *};
use tokuly_video::{
    channels::{self, ChannelRow},
    videos,
};

use crate::{
    Live,
    access::{self, Viewer},
    chat::{self, ChatRow},
    streams::{self, StreamRow},
};

/// Keyset of the restriction list: newest first.
#[derive(Debug, Serialize, Deserialize)]
struct RestrictionCursor {
    at: Time,
    user_id: Uuid,
}

fn channel_not_found() -> AppError {
    AppError::not_found("CHANNEL_NOT_FOUND", "channel not found")
}

fn message_not_found() -> AppError {
    AppError::not_found("MESSAGE_NOT_FOUND", "message not found")
}

impl Live {
    /// Moderation acts are pushed to everyone watching; one account may
    /// perform only so many a minute.
    async fn limit_moderation(&self, user_id: Uuid) -> AppResult<()> {
        self.core.cache.rate_limit(&format!("live-moderate:{user_id}"), 120, Duration::from_secs(60)).await
    }

    /// A channel named by a client, for someone who claims to moderate it.
    async fn moderated_channel(&self, user_id: Uuid, reference: &str) -> AppResult<(ChannelRow, access::Standing)> {
        let channel = channels::find(&self.core, reference).await?.ok_or_else(channel_not_found)?;
        let standing = access::require_moderator(&self.core, channel.id, channel.owner_id, user_id).await?;
        Ok((channel, standing))
    }

    /// A message with its stream, if `viewer` may see the stream.
    async fn visible_message(&self, viewer: &Viewer, message_id: &str) -> AppResult<(ChatRow, StreamRow)> {
        let Ok(message_id) = Uuid::parse_str(message_id.trim()) else { return Err(message_not_found()) };
        let message = chat::find(&self.core, message_id).await?.ok_or_else(message_not_found)?;
        let stream = streams::find_by_id(&self.core, message.stream_id).await?.ok_or_else(message_not_found)?;
        ensure!(
            access::can_watch_now(&self.core, viewer, &stream.facts()).await?,
            not_found,
            "MESSAGE_NOT_FOUND",
            "message not found"
        );
        Ok((message, stream))
    }
}

impl LiveChatService for Live {
    async fn send_chat_message(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SendChatMessageRequest>,
    ) -> ServiceResult<SendChatMessageResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let stream = streams::find_watchable(core, &viewer, request.stream_id).await?;
            let stamp_id = parse_opt_id(request.stamp_id.trim())?;
            let row = self.post_chat_message(&stream, user_id, request.text, stamp_id).await?;
            ok(SendChatMessageResponse {
                message: Some(chat::to_proto_one(core, &stream, &row).await?).into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_chat_messages(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListChatMessagesRequest>,
    ) -> ServiceResult<ListChatMessagesResponse> {
        handle(async {
            let core = &self.core;
            let viewer = Viewer::load(core, ctx.maybe_user_id()?).await?;
            let stream = streams::find_watchable(core, &viewer, request.stream_id).await?;
            let page = Page::from_view(request.page.as_option(), 200);
            let before: Option<Uuid> = page.decode()?;
            let mut rows = chat::list(core, stream.id, before, &viewer.blocked, page.fetch()).await?;
            let info = page.finish(&mut rows, |row| row.id);
            ok(ListChatMessagesResponse {
                messages: chat::to_protos(core, &stream, &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_chat_replay(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListChatReplayRequest>,
    ) -> ServiceResult<ListChatReplayResponse> {
        handle(async {
            let core = &self.core;
            let viewer = Viewer::load(core, ctx.maybe_user_id()?).await?;
            let stream = if !request.stream_id.trim().is_empty() {
                streams::find_watchable(core, &viewer, request.stream_id).await?
            } else {
                // Whoever may watch the archive may read the chat that went with it.
                let video =
                    videos::find_ref(core, request.video_id).await?.filter(|video| viewer.can_watch(&video.facts()));
                let stream_id = video.and_then(|video| video.origin_id).ok_or_else(streams::not_found)?;
                streams::find_by_id(core, stream_id).await?.ok_or_else(streams::not_found)?
            };
            let caller = viewer.user_id.map(|id| id.to_string()).unwrap_or_else(|| ctx.client().ip);
            core.cache.rate_limit(&format!("live-replay:{caller}"), 240, Duration::from_secs(60)).await?;
            let (from, to) = (request.from_offset_ms, request.to_offset_ms);
            ensure!(
                from >= 0 && to > from && to - from <= chat::MAX_REPLAY_SPAN_MS,
                invalid,
                "REPLAY_RANGE_INVALID",
                "ask for up to ten minutes of chat at a time"
            );
            let mut rows = chat::replay(core, stream.id, from, to, &viewer.blocked, chat::REPLAY_LIMIT + 1).await?;
            let truncated = rows.len() as i64 > chat::REPLAY_LIMIT;
            rows.truncate(chat::REPLAY_LIMIT as usize);
            ok(ListChatReplayResponse {
                messages: chat::to_protos(core, &stream, &rows).await?,
                truncated,
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_chat_message(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteChatMessageRequest>,
    ) -> ServiceResult<DeleteChatMessageResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let (message, stream) = self.visible_message(&viewer, request.message_id).await?;
            let standing = access::standing(core, stream.channel_id, stream.owner_id, user_id).await?;
            // People delete their own lines; a gift's line is a receipt and stays unless a moderator removes it.
            let own = message.user_id == Some(user_id) && message.kind != chat::KIND_GIFT;
            ensure!(
                standing.can_moderate() || own,
                denied,
                "NOT_A_MODERATOR",
                "only the streamer and moderators can delete this message"
            );
            // Moderators answer to the streamer: what the streamer wrote is not theirs to remove.
            ensure!(
                standing.is_owner || own || message.user_id != Some(stream.owner_id),
                denied,
                "CANNOT_MODERATE_STREAMER",
                "moderators cannot remove the streamer's messages"
            );
            self.limit_moderation(user_id).await?;
            self.remove_chat_message(&stream, &message).await?;
            ok(DeleteChatMessageResponse::default())
        })
        .await
    }

    async fn pin_chat_message(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, PinChatMessageRequest>,
    ) -> ServiceResult<PinChatMessageResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let (message, stream) = self.visible_message(&viewer, request.message_id).await?;
            access::require_moderator(core, stream.channel_id, stream.owner_id, user_id).await?;
            ensure!(message.kind != chat::KIND_NOTICE, not_found, "MESSAGE_NOT_FOUND", "message not found");
            self.limit_moderation(user_id).await?;
            let pinned = self.set_pinned_message(&stream, Some(&message)).await?;
            ok(PinChatMessageResponse { message: pinned.into(), ..Default::default() })
        })
        .await
    }

    async fn unpin_chat_message(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnpinChatMessageRequest>,
    ) -> ServiceResult<UnpinChatMessageResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let stream = streams::find_watchable(core, &viewer, request.stream_id).await?;
            access::require_moderator(core, stream.channel_id, stream.owner_id, user_id).await?;
            if stream.pinned_message_id.is_some() {
                self.limit_moderation(user_id).await?;
                self.set_pinned_message(&stream, None).await?;
            }
            ok(UnpinChatMessageResponse::default())
        })
        .await
    }

    async fn update_chat_settings(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateChatSettingsRequest>,
    ) -> ServiceResult<UpdateChatSettingsResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let stream = streams::find_watchable(core, &viewer, &req.stream_id).await?;
            // Who may write, how fast and whether at all is the streamer's
            // call alone; moderators act on messages and people, not on the chat.
            ensure!(
                viewer.is(stream.owner_id),
                denied,
                "NOT_THE_STREAMER",
                "only the streamer changes the chat's rules"
            );
            let settings = req
                .chat
                .as_option()
                .ok_or_else(|| AppError::invalid("CHAT_SETTINGS_INVALID", "the new chat settings are missing"))?;
            let settings = streams::validate_chat_settings(settings)?;
            self.limit_moderation(user_id).await?;
            sqlx::query("UPDATE live_streams SET chat = $2 WHERE id = $1")
                .bind(stream.id)
                .bind(streams::chat_json(&settings)?)
                .execute(&core.db)
                .await?;
            let change = ChatSettingsChanged {
                stream_id: stream.id.to_string(),
                chat: Some(settings.clone()).into(),
                ..Default::default()
            };
            self.push_to_stream(stream.id, Event::ChatSettingsChanged(Box::new(change))).await;
            ok(UpdateChatSettingsResponse { chat: Some(settings).into(), ..Default::default() })
        })
        .await
    }

    async fn get_banned_words(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetBannedWordsRequest>,
    ) -> ServiceResult<GetBannedWordsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let (channel, _) = self.moderated_channel(user_id, request.channel_id).await?;
            ok(GetBannedWordsResponse {
                words: chat::banned_words(&self.core, channel.id).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn set_banned_words(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetBannedWordsRequest>,
    ) -> ServiceResult<SetBannedWordsResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let channel = channels::require_owned_ref(core, user_id, &req.channel_id).await?;
            let words = chat::normalize_banned_words(&req.words)?;
            self.limit_moderation(user_id).await?;
            sqlx::query(
                "INSERT INTO live_channel_settings (channel_id, owner_id, banned_words) VALUES ($1, $2, $3)
                 ON CONFLICT (channel_id) DO UPDATE SET banned_words = EXCLUDED.banned_words, updated_at = now()",
            )
            .bind(channel.id)
            .bind(channel.owner_id)
            .bind(&words)
            .execute(&core.db)
            .await?;
            ok(SetBannedWordsResponse { words, ..Default::default() })
        })
        .await
    }

    async fn restrict_chat_user(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RestrictChatUserRequest>,
    ) -> ServiceResult<RestrictChatUserResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let (channel, standing) = self.moderated_channel(user_id, request.channel_id).await?;
            let target_id = parse_id(request.user_id)?;
            self.limit_moderation(user_id).await?;
            let restriction = self
                .restrict_user(
                    &channel,
                    user_id,
                    standing,
                    target_id,
                    request.duration_seconds,
                    request.reason,
                    request.delete_messages,
                )
                .await?;
            ok(RestrictChatUserResponse { restriction: Some(restriction).into(), ..Default::default() })
        })
        .await
    }

    async fn unrestrict_chat_user(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnrestrictChatUserRequest>,
    ) -> ServiceResult<UnrestrictChatUserResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let (channel, _) = self.moderated_channel(user_id, request.channel_id).await?;
            self.limit_moderation(user_id).await?;
            self.unrestrict_user(channel.id, parse_id(request.user_id)?).await?;
            ok(UnrestrictChatUserResponse::default())
        })
        .await
    }

    async fn list_chat_restrictions(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListChatRestrictionsRequest>,
    ) -> ServiceResult<ListChatRestrictionsResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let (channel, _) = self.moderated_channel(user_id, request.channel_id).await?;
            let page = Page::from_view(request.page.as_option(), 100);
            let after: Option<RestrictionCursor> = page.decode()?;
            let mut rows: Vec<(Uuid, Option<Time>, String, Option<Uuid>, Time)> = sqlx::query_as(
                "SELECT user_id, expires_at, reason, moderator_id, created_at FROM live_chat_restrictions
                 WHERE channel_id = $1 AND (expires_at IS NULL OR expires_at > now())
                   AND ($2::timestamptz IS NULL OR (created_at, user_id) < ($2, $3))
                 ORDER BY created_at DESC, user_id DESC LIMIT $4",
            )
            .bind(channel.id)
            .bind(after.as_ref().map(|cursor| cursor.at))
            .bind(after.as_ref().map(|cursor| cursor.user_id))
            .bind(page.fetch())
            .fetch_all(&core.db)
            .await?;
            let info = page.finish(&mut rows, |(user_id, _, _, _, created_at)| RestrictionCursor {
                at: *created_at,
                user_id: *user_id,
            });
            let mut ids: Vec<Uuid> = rows
                .iter()
                .flat_map(|(user_id, _, _, moderator_id, _)| [Some(*user_id), *moderator_id])
                .flatten()
                .collect();
            ids.sort_unstable();
            ids.dedup();
            let users = tokuly_identity::summaries(core, &ids).await?;
            let restrictions = rows
                .into_iter()
                .map(|(restricted, expires_at, reason, moderator_id, created_at)| ChatRestriction {
                    user: Some(tokuly_identity::users::pick(&users, restricted)).into(),
                    expires_at: opt_ts(expires_at),
                    reason,
                    // The streamer acts as the channel and is not named.
                    moderator: moderator_id
                        .filter(|id| *id != channel.owner_id)
                        .and_then(|id| users.get(&id).cloned())
                        .into(),
                    created_at: ts(created_at),
                    ..Default::default()
                })
                .collect();
            ok(ListChatRestrictionsResponse { restrictions, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn add_moderator(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AddModeratorRequest>,
    ) -> ServiceResult<AddModeratorResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let channel = channels::require_owned_ref(&self.core, user_id, request.channel_id).await?;
            // An appointment notifies its target.
            self.core.cache.rate_limit(&format!("live-appoint:{user_id}"), 20, Duration::from_secs(3600)).await?;
            let moderator = self.appoint_moderator(&channel, parse_id(request.user_id)?).await?;
            ok(AddModeratorResponse { moderator: Some(moderator).into(), ..Default::default() })
        })
        .await
    }

    async fn remove_moderator(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveModeratorRequest>,
    ) -> ServiceResult<RemoveModeratorResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let channel = channels::find(core, request.channel_id).await?.ok_or_else(channel_not_found)?;
            let target_id = parse_id(request.user_id)?;
            // The streamer dismisses; a moderator may step down.
            ensure!(
                channel.owner_id == user_id || target_id == user_id,
                not_found,
                "CHANNEL_NOT_FOUND",
                "channel not found"
            );
            sqlx::query("DELETE FROM live_moderators WHERE channel_id = $1 AND user_id = $2")
                .bind(channel.id)
                .bind(target_id)
                .execute(&core.db)
                .await?;
            ok(RemoveModeratorResponse::default())
        })
        .await
    }

    async fn list_moderators(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListModeratorsRequest>,
    ) -> ServiceResult<ListModeratorsResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let (channel, _) = self.moderated_channel(user_id, request.channel_id).await?;
            let rows: Vec<(Uuid, Time)> = sqlx::query_as(
                "SELECT user_id, created_at FROM live_moderators WHERE channel_id = $1 ORDER BY created_at, user_id",
            )
            .bind(channel.id)
            .fetch_all(&core.db)
            .await?;
            let ids: Vec<Uuid> = rows.iter().map(|(id, _)| *id).collect();
            let users = tokuly_identity::summaries(core, &ids).await?;
            let moderators = rows
                .into_iter()
                .map(|(id, appointed_at)| Moderator {
                    user: Some(tokuly_identity::users::pick(&users, id)).into(),
                    appointed_at: ts(appointed_at),
                    ..Default::default()
                })
                .collect();
            ok(ListModeratorsResponse { moderators, ..Default::default() })
        })
        .await
    }
}
