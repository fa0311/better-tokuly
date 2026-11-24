//! `LiveStreamService`: creating, running, finding and watching streams.
use std::{collections::HashMap, time::Duration};

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use serde::{Deserialize, Serialize};
use tokuly_core::{analytics::AnalyticsEvent, prelude::*};
use tokuly_proto::{
    common::Visibility,
    live::{live_event::Event, *},
};
use tokuly_video::{channels, validate as video_validate, videos};

use crate::{
    Live,
    access::{self, Viewer, in_feed, listed, public_stream},
    audience::{self, ViewerKey},
    chat::{self, NewMessage},
    polls,
    streams::{self, StreamEdit, StreamRow, stream_columns},
};

/// Streams looked at for the subscription list (see `list_subscribed_streams`).
const SUBSCRIBED_CANDIDATES: i64 = 1000;

/// SQL: stream `s` is on air, or scheduled and not long overdue.
const ON_AIR_OR_SCHEDULED: &str =
    "(s.status = 2 OR (s.status = 1 AND s.scheduled_at IS NOT NULL AND s.scheduled_at > now() - interval '1 hour'))";
/// SQL: on air first (most viewers first), then the scheduled ones (soonest first).
const ON_AIR_FIRST: &str = "s.status DESC, s.ranked_viewer_count DESC, s.scheduled_at NULLS LAST, s.id";

/// Keyset of the "live now" directory.
#[derive(Debug, Serialize, Deserialize)]
struct LiveCursor {
    viewers: i64,
    id: Uuid,
}

/// Keyset of the upcoming directory.
#[derive(Debug, Serialize, Deserialize)]
struct UpcomingCursor {
    at: Time,
    id: Uuid,
}

fn explicit_visibility(value: buffa::EnumValue<Visibility>) -> AppResult<i32> {
    match value.as_known() {
        Some(Visibility::VISIBILITY_UNSPECIFIED) | None => {
            Err(AppError::invalid("VISIBILITY_INVALID", "unknown visibility"))
        }
        Some(known) => Ok(known as i32),
    }
}

fn status_filter(value: buffa::EnumValue<StreamStatus>) -> AppResult<i32> {
    ensure!(value.as_known().is_some(), invalid, "STATUS_INVALID", "unknown stream status");
    Ok(value.to_i32())
}

impl Live {
    /// The stream page as `viewer` sees it, who must already have been
    /// allowed to open it.
    ///
    /// `session` says whether the caller is signed in with a session. An
    /// API token is never handed a playback token (see [`Live::playback_for`]).
    pub(crate) async fn stream_proto(&self, viewer: &Viewer, row: &StreamRow, session: bool) -> AppResult<Stream> {
        let core = &self.core;
        let is_owner = viewer.is(row.owner_id);
        let settings = row.chat_settings();
        // Viewers play a stream on air; the owner previews as soon as the encoder is there.
        let playable = self.cfg.media_server_configured()
            && (row.is_live() || (is_owner && row.encoder_connected && !row.is_ended()));
        let pinned = match chat::pinned(core, row).await? {
            Some(message) => Some(chat::to_proto_one(core, row, &message).await?),
            None => None,
        };
        let highlighted = chat::to_protos(core, row, &chat::highlighted(core, row.id).await?).await?;
        let poll = match polls::open_of(core, row.id).await? {
            Some(poll) => Some(polls::to_proto_for(core, &poll, viewer.user_id).await?),
            None => None,
        };
        let archive_video = match row.archive_video_id {
            Some(video_id) => videos::find_by_id(core, video_id).await?.map(|video| video.short_id),
            None => None,
        };
        let viewer_state = match viewer.user_id {
            None => None,
            Some(user_id) => {
                let standing = access::standing(core, row.channel_id, row.owner_id, user_id).await?;
                let reminder_set: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM live_reminders WHERE stream_id = $1 AND user_id = $2)",
                )
                .bind(row.id)
                .bind(user_id)
                .fetch_one(&core.db)
                .await?;
                let block = chat::chat_block(core, row, &settings, user_id, standing).await?;
                Some(StreamViewerState {
                    is_owner,
                    is_moderator: standing.can_moderate(),
                    reminder_set,
                    subscribed: channels::is_subscribed(core, user_id, row.channel_id).await?,
                    can_chat: block.is_none(),
                    chat_block_reason: block.map(|b| b.reason.to_owned()).unwrap_or_default(),
                    chat_blocked_until: opt_ts(block.and_then(|b| b.until)),
                    ..Default::default()
                })
            }
        };
        let studio = is_owner.then(|| StreamStudioInfo {
            encoder_connected: row.encoder_connected,
            encoder_protocol: row.encoder_protocol.clone(),
            auto_start: row.auto_start,
            gift_coins: row.gift_coins,
            gift_count: row.gift_count,
            chat_message_count: row.chat_message_count,
            new_subscribers: row.new_subscribers,
            archive_error: row.archive_error.clone(),
            end_reason: row.end_reason.into(),
            circle_id: row.circle_id.map(|id| id.to_string()).unwrap_or_default(),
            ..Default::default()
        });
        let mut summary = streams::summary(core, row).await?;
        if is_owner {
            summary.visibility = row.visibility.into();
        }
        Ok(Stream {
            summary: Some(summary).into(),
            description: row.description.clone(),
            tags: row.tags.clone(),
            dvr_enabled: row.dvr_enabled,
            auto_archive: row.auto_archive,
            gifts_enabled: row.gifts_enabled,
            chat: Some(settings).into(),
            playback: playable.then(|| self.playback_for(row, viewer.user_id, session)).flatten().into(),
            pinned_message: pinned.into(),
            highlighted_gifts: highlighted,
            active_poll: poll.into(),
            archive_status: row.archive_status.into(),
            archive_video_id: archive_video.unwrap_or_default(),
            raid_target_stream_id: row.raid_target_id.map(|id| id.to_string()).unwrap_or_default(),
            viewer: viewer_state.into(),
            studio: studio.into(),
            realtime_topic: row.topic(),
            report_interval_seconds: self.cfg.report_interval_seconds as i32,
            ..Default::default()
        })
    }

    /// The page of a stream of `owner_id`, as they see it.
    async fn own_stream_proto(&self, owner: &AuthContext, row: &StreamRow) -> AppResult<Stream> {
        let viewer = Viewer { user_id: Some(owner.user_id), ..Default::default() };
        self.stream_proto(&viewer, row, owner.is_session()).await
    }
}

/// One page of `rows` by position, for lists that are narrowed down in code.
fn page_by_position(rows: Vec<StreamRow>, page: &Page) -> AppResult<(Vec<StreamRow>, tokuly_proto::common::PageInfo)> {
    let skip: usize = page.decode()?.unwrap_or(0);
    let mut slice: Vec<(usize, StreamRow)> =
        rows.into_iter().enumerate().skip(skip).take(page.fetch() as usize).collect();
    let info = page.finish(&mut slice, |(position, _)| position + 1);
    Ok((slice.into_iter().map(|(_, stream)| stream).collect(), info))
}

impl LiveStreamService for Live {
    async fn create_stream(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateStreamRequest>,
    ) -> ServiceResult<CreateStreamResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let channel = if req.channel_id.trim().is_empty() {
                channels::default_for(core, user_id).await?.ok_or_else(|| {
                    AppError::precondition("CHANNEL_REQUIRED", "create a channel before your first stream")
                })?
            } else {
                channels::require_owned_ref(core, user_id, &req.channel_id).await?
            };
            let edit = StreamEdit {
                title: Some(req.title),
                description: Some(req.description),
                tags: Some(req.tags),
                category: Some(video_validate::category(req.category)?),
                game: Some(req.game),
                thumbnail_asset_id: Some(parse_opt_id(req.thumbnail_asset_id.trim())?),
                visibility: Some(video_validate::visibility(req.visibility, Visibility::VISIBILITY_PUBLIC)?),
                circle_id: parse_opt_id(&req.circle_id)?,
                scheduled_at: Some(req.scheduled_at.as_option().map(tokuly_core::time::from_ts)),
                low_latency: req.low_latency,
                dvr_enabled: req.dvr_enabled,
                auto_archive: req.auto_archive,
                auto_start: req.auto_start,
                gifts_enabled: req.gifts_enabled,
                chat: req.chat.into_option(),
            };
            core.cache.rate_limit(&format!("live-create:{user_id}"), 30, Duration::from_secs(3600)).await?;
            let row = streams::create(core, &channel, self.cfg.max_open_streams, edit).await?;
            // Locked or unlocked since `create` read it: apply it now.
            let row = if tokuly_video::access::is_locked_now(core, user_id).await? != row.owner_locked {
                self.sync_owner_lock(user_id).await?;
                streams::find_by_id(core, row.id).await?.ok_or_else(streams::not_found)?
            } else {
                row
            };
            ok(CreateStreamResponse {
                stream: Some(self.own_stream_proto(ctx.auth()?, &row).await?).into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_stream(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateStreamRequest>,
    ) -> ServiceResult<UpdateStreamResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let stream = streams::find_owned(core, user_id, &req.stream_id).await?;
            let thumbnail_asset_id = match req.thumbnail_asset_id.as_deref().map(str::trim) {
                None => None,
                Some("") => Some(None),
                Some(id) => Some(Some(parse_id(id)?)),
            };
            let scheduled_at = match (req.clear_schedule, req.scheduled_at.as_option()) {
                (true, _) => Some(None),
                (false, Some(at)) => Some(Some(tokuly_core::time::from_ts(at))),
                (false, None) => None,
            };
            let edit = StreamEdit {
                title: req.title,
                description: req.description,
                tags: req.tags.into_option().map(|list| list.tags),
                category: req.category.map(video_validate::category).transpose()?,
                game: req.game,
                thumbnail_asset_id,
                visibility: req.visibility.map(explicit_visibility).transpose()?,
                circle_id: parse_opt_id(&req.circle_id)?,
                scheduled_at,
                low_latency: req.low_latency,
                dvr_enabled: req.dvr_enabled,
                auto_archive: req.auto_archive,
                auto_start: req.auto_start,
                gifts_enabled: req.gifts_enabled,
                chat: None,
            };
            // Every edit is pushed to the stream's viewers and widgets.
            core.cache.rate_limit(&format!("live-update:{user_id}"), 60, Duration::from_secs(60)).await?;
            let updated = streams::update(core, &stream, edit).await?;
            if !updated.is_ended() {
                self.push_stream_updated(&updated).await;
            }
            let (was, now) = (stream.audience(stream.owner_locked), updated.audience(updated.owner_locked));
            if access::narrower(was, now) {
                // Whoever the new visibility leaves out stops following the stream.
                // (Widening, or moving between the ones open to everyone, leaves nobody out.)
                core.rt.recheck_topic(&updated.topic()).await;
            }
            // Narrower than before: whoever may no longer watch is cut off now, not when their token runs out.
            if was != now && (updated.is_live() || updated.encoder_connected) {
                let cut = async {
                    self.access_changed(&[updated.owner_id]).await?;
                    self.enforce_access(&updated, true).await
                };
                if let Err(error) = cut.await {
                    tracing::warn!(stream_id = %updated.id, %error, "could not cut viewers after a visibility change");
                }
            }
            ok(UpdateStreamResponse {
                stream: Some(self.own_stream_proto(ctx.auth()?, &updated).await?).into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_stream(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteStreamRequest>,
    ) -> ServiceResult<DeleteStreamResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let stream = streams::find_owned(core, user_id, request.stream_id).await?;
            // Viewers are told, the encoder is sent away; no archive is made of a deleted stream.
            self.finish_stream(stream.id, EndReason::END_REASON_STREAMER, None, Some(user_id)).await?;
            let mut tx = core.db.begin().await?;
            let current = streams::lock(&mut tx, stream.id).await?.ok_or_else(streams::not_found)?;
            let aftermath = streams::delete_in(&mut tx, &current, Some(user_id)).await?;
            tx.commit().await?;
            self.clean_up(aftermath).await;
            ok(DeleteStreamResponse::default())
        })
        .await
    }

    async fn get_stream(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetStreamRequest>,
    ) -> ServiceResult<GetStreamResponse> {
        handle(async {
            let viewer = Viewer::load(&self.core, ctx.maybe_user_id()?).await?;
            let session = ctx.maybe_auth()?.is_none_or(|auth| auth.is_session());
            let stream = streams::find_watchable(&self.core, &viewer, request.stream_id).await?;
            ok(GetStreamResponse {
                stream: Some(self.stream_proto(&viewer, &stream, session).await?).into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_channel_streams(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListChannelStreamsRequest>,
    ) -> ServiceResult<ListChannelStreamsResponse> {
        handle(async {
            let core = &self.core;
            let viewer = Viewer::load(core, ctx.maybe_user_id()?).await?;
            let channel = channels::find_visible(core, &viewer, request.channel_id).await?;
            let status = status_filter(request.status)?;
            let page = Page::from_view(request.page.as_option(), 100);
            let before: Option<Uuid> = page.decode()?;
            let mut rows: Vec<StreamRow> = if viewer.is(channel.owner_id) {
                sqlx::query_as(concat!(
                    "SELECT ",
                    stream_columns!(),
                    " FROM live_streams s
                      WHERE s.channel_id = $1 AND s.deleted_at IS NULL AND ($2 = 0 OR s.status = $2)
                        AND ($3::uuid IS NULL OR s.id < $3)
                      ORDER BY s.id DESC LIMIT $4"
                ))
                .bind(channel.id)
                .bind(status)
                .bind(before)
                .bind(page.fetch())
                .fetch_all(&core.db)
                .await?
            } else {
                // Streams that were cancelled before they went on air are the owner's business.
                viewer
                    .query_as(concat!(
                        "SELECT ",
                        stream_columns!(),
                        " FROM live_streams s
                          WHERE s.channel_id = $7 AND s.deleted_at IS NULL AND ($8 = 0 OR s.status = $8)
                            AND ($9::uuid IS NULL OR s.id < $9) AND NOT (s.status = 3 AND s.started_at IS NULL) AND ",
                        listed!(),
                        " ORDER BY s.id DESC LIMIT $10"
                    ))
                    .bind(channel.id)
                    .bind(status)
                    .bind(before)
                    .bind(page.fetch())
                    .fetch_all(&core.db)
                    .await?
            };
            let info = page.finish(&mut rows, |row| row.id);
            viewer.drop_newly_locked(core, &mut rows, StreamRow::lock_facts).await?;
            ok(ListChannelStreamsResponse {
                streams: streams::summaries(core, &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_live_streams(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListLiveStreamsRequest>,
    ) -> ServiceResult<ListLiveStreamsResponse> {
        handle(async {
            let core = &self.core;
            let viewer = Viewer::load(core, ctx.maybe_user_id()?).await?;
            let category = video_validate::category(request.category)?;
            let game = request.game.split_whitespace().collect::<Vec<_>>().join(" ");
            let page = Page::from_view(request.page.as_option(), 100);
            let after: Option<LiveCursor> = page.decode()?;
            let mut rows: Vec<StreamRow> = viewer
                .query_as(concat!(
                    "SELECT ",
                    stream_columns!(),
                    " FROM live_streams s
                  WHERE s.status = 2 AND ",
                    public_stream!(),
                    " AND ($7 = 0 OR s.category = $7) AND ($8 = '' OR lower(s.game) = lower($8))
                    AND ($9::bigint IS NULL OR (s.ranked_viewer_count, s.id) < ($9, $10))
                  ORDER BY s.ranked_viewer_count DESC, s.id DESC LIMIT $11"
                ))
                .bind(category)
                .bind(&game)
                .bind(after.as_ref().map(|cursor| cursor.viewers))
                .bind(after.as_ref().map(|cursor| cursor.id))
                .bind(page.fetch())
                .fetch_all(&core.db)
                .await?;
            let info = page.finish(&mut rows, |row| LiveCursor { viewers: row.ranked_viewer_count, id: row.id });
            viewer.drop_newly_locked(core, &mut rows, StreamRow::lock_facts).await?;
            ok(ListLiveStreamsResponse {
                streams: streams::summaries(core, &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_upcoming_streams(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListUpcomingStreamsRequest>,
    ) -> ServiceResult<ListUpcomingStreamsResponse> {
        handle(async {
            let core = &self.core;
            let viewer = Viewer::load(core, ctx.maybe_user_id()?).await?;
            let category = video_validate::category(request.category)?;
            let page = Page::from_view(request.page.as_option(), 100);
            let after: Option<UpcomingCursor> = page.decode()?;
            let mut rows: Vec<StreamRow> = viewer.query_as(concat!(
                "SELECT ",
                stream_columns!(),
                " FROM live_streams s
                  WHERE s.status = 1 AND s.scheduled_at IS NOT NULL AND s.scheduled_at > now() - interval '1 hour' AND ",
                public_stream!(),
                " AND ($7 = 0 OR s.category = $7) AND ($8::timestamptz IS NULL OR (s.scheduled_at, s.id) > ($8, $9))
                  ORDER BY s.scheduled_at, s.id LIMIT $10"
            ))
            .bind(category)
            .bind(after.as_ref().map(|cursor| cursor.at))
            .bind(after.as_ref().map(|cursor| cursor.id))
            .bind(page.fetch())
            .fetch_all(&core.db)
            .await?;
            let info = page.finish(&mut rows, |row| UpcomingCursor { at: row.scheduled_at.unwrap_or(row.created_at), id: row.id });
            viewer.drop_newly_locked(core, &mut rows, StreamRow::lock_facts).await?;
            ok(ListUpcomingStreamsResponse { streams: streams::summaries(core, &rows).await?, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn list_subscribed_streams(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListSubscribedStreamsRequest>,
    ) -> ServiceResult<ListSubscribedStreamsResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let page = Page::from_view(request.page.as_option(), 100);
            core.cache.rate_limit(&format!("live-lists:{user_id}"), 60, Duration::from_secs(60)).await?;
            // Subscriptions belong to the video domain, which answers "is
            // this user subscribed to this channel": the streams that could
            // be listed are read first and then narrowed down channel by
            // channel.
            let mut candidates: Vec<StreamRow> = viewer
                .query_as(sqlx::AssertSqlSafe(format!(
                    "SELECT {} FROM live_streams s WHERE {ON_AIR_OR_SCHEDULED} AND s.owner_id <> $1 AND {} ORDER BY {ON_AIR_FIRST} LIMIT $7",
                    stream_columns!(),
                    in_feed!(),
                )))
                .bind(SUBSCRIBED_CANDIDATES)
                .fetch_all(&core.db)
                .await?;
            viewer.drop_newly_locked(core, &mut candidates, StreamRow::lock_facts).await?;
            let mut subscribed: HashMap<Uuid, bool> = HashMap::new();
            let mut kept = Vec::new();
            for stream in candidates {
                let is_subscribed = match subscribed.get(&stream.channel_id) {
                    Some(known) => *known,
                    None => {
                        let answer = channels::is_subscribed(core, user_id, stream.channel_id).await?;
                        subscribed.insert(stream.channel_id, answer);
                        answer
                    }
                };
                if is_subscribed {
                    kept.push(stream);
                }
            }
            let (rows, info) = page_by_position(kept, &page)?;
            ok(ListSubscribedStreamsResponse { streams: streams::summaries(core, &rows).await?, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn list_friend_streams(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListFriendStreamsRequest>,
    ) -> ServiceResult<ListFriendStreamsResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let page = Page::from_view(request.page.as_option(), 100);
            let candidates: Vec<StreamRow> = viewer
                .query_as(sqlx::AssertSqlSafe(format!(
                    "SELECT {} FROM live_streams s
                     WHERE {ON_AIR_OR_SCHEDULED} AND s.owner_id = ANY($2) AND {} ORDER BY {ON_AIR_FIRST} LIMIT $7",
                    stream_columns!(),
                    in_feed!(),
                )))
                .bind(SUBSCRIBED_CANDIDATES)
                .fetch_all(&core.db)
                .await?;
            let (rows, info) = page_by_position(candidates, &page)?;
            ok(ListFriendStreamsResponse {
                streams: streams::summaries(core, &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_ingest_info(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetIngestInfoRequest>,
    ) -> ServiceResult<GetIngestInfoResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let stream = streams::find_owned(&self.core, user_id, request.stream_id).await?;
            ensure!(!stream.is_ended(), precondition, "STREAM_ENDED", "the stream is over");
            self.require_media_server()?;
            ok(GetIngestInfoResponse { ingest: Some(self.ingest_info(&stream)?).into(), ..Default::default() })
        })
        .await
    }

    async fn rotate_stream_key(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RotateStreamKeyRequest>,
    ) -> ServiceResult<RotateStreamKeyResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let stream = streams::find_owned(core, user_id, request.stream_id).await?;
            ensure!(!stream.is_ended(), precondition, "STREAM_ENDED", "the stream is over");
            core.cache.rate_limit(&format!("live-key:{user_id}"), 10, Duration::from_secs(3600)).await?;
            let (_, sealed, hash) = streams::new_stream_key(core);
            sqlx::query("UPDATE live_streams SET stream_key_sealed = $2, stream_key_hash = $3, key_rotated_at = now() WHERE id = $1")
                .bind(stream.id)
                .bind(&sealed)
                .bind(&hash)
                .execute(&core.db)
                .await?;
            // Whoever publishes with the old key is sent away.
            self.kick_quietly(&stream).await;
            let rotated = streams::find_by_id(core, stream.id).await?.ok_or_else(streams::not_found)?;
            ok(RotateStreamKeyResponse { ingest: Some(self.ingest_info(&rotated)?).into(), ..Default::default() })
        })
        .await
    }

    async fn start_stream(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, StartStreamRequest>,
    ) -> ServiceResult<StartStreamResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let mut stream = streams::find_owned(core, user_id, request.stream_id).await?;
            ensure!(!stream.is_ended(), precondition, "STREAM_ENDED", "the stream is over");
            if stream.is_upcoming() {
                self.require_media_server()?;
                core.cache.rate_limit(&format!("live-start:{user_id}"), 30, Duration::from_secs(60)).await?;
                if !stream.encoder_connected {
                    // The reconciler may not have looked yet: ask the media server now.
                    if let Some(path) = self.media_path(&stream.media_path()).await?.filter(|path| path.online) {
                        self.encoder_connected(&stream, path.protocol()).await?;
                        stream = streams::find_by_id(core, stream.id).await?.ok_or_else(streams::not_found)?;
                    }
                }
                if stream.is_upcoming() {
                    ensure!(
                        stream.encoder_connected,
                        precondition,
                        "ENCODER_NOT_CONNECTED",
                        "start your encoder first: the stream goes on air once it is publishing"
                    );
                    self.go_live(stream.id).await?;
                }
                stream = streams::find_by_id(core, stream.id).await?.ok_or_else(streams::not_found)?;
            }
            ok(StartStreamResponse {
                stream: Some(self.own_stream_proto(ctx.auth()?, &stream).await?).into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn end_stream(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, EndStreamRequest>,
    ) -> ServiceResult<EndStreamResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let stream = streams::find_owned(core, user_id, request.stream_id).await?;
            self.finish_stream(stream.id, EndReason::END_REASON_STREAMER, None, Some(user_id)).await?;
            let ended = streams::find_by_id(core, stream.id).await?.ok_or_else(streams::not_found)?;
            ok(EndStreamResponse {
                stream: Some(self.own_stream_proto(ctx.auth()?, &ended).await?).into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn set_reminder(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetReminderRequest>,
    ) -> ServiceResult<SetReminderResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let stream = streams::find_watchable(core, &viewer, request.stream_id).await?;
            if request.enabled {
                ensure!(
                    stream.is_upcoming(),
                    precondition,
                    "STREAM_ALREADY_STARTED",
                    "this stream has already started"
                );
                core.cache.rate_limit(&format!("live-reminder:{user_id}"), 60, Duration::from_secs(60)).await?;
                sqlx::query("INSERT INTO live_reminders (stream_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                    .bind(stream.id)
                    .bind(user_id)
                    .execute(&core.db)
                    .await?;
            } else {
                sqlx::query("DELETE FROM live_reminders WHERE stream_id = $1 AND user_id = $2")
                    .bind(stream.id)
                    .bind(user_id)
                    .execute(&core.db)
                    .await?;
            }
            ok(SetReminderResponse { enabled: request.enabled, ..Default::default() })
        })
        .await
    }

    async fn report_watching(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ReportWatchingRequest>,
    ) -> ServiceResult<ReportWatchingResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.maybe_user_id()?;
            let session = ctx.maybe_auth()?.is_none_or(|auth| auth.is_session());
            let viewer = Viewer::load(core, user_id).await?;
            let stream = streams::find_watchable(core, &viewer, request.stream_id).await?;
            ensure!(
                request.client_id.len() <= 64,
                invalid,
                "CLIENT_ID_INVALID",
                "the client ID is at most 64 characters"
            );
            let key = match user_id {
                Some(user_id) => ViewerKey::User(user_id),
                None => ViewerKey::guest(core, request.client_id, &ctx.client().ip),
            };
            let interval = self.cfg.report_interval_seconds;
            core.cache
                .rate_limit(&format!("live-watch:{}:{}", stream.id, key.member()), 20, Duration::from_secs(60))
                .await?;
            if stream.is_live() {
                if viewer.is(stream.owner_id) {
                    // The streamer's own player is their dashboard, not an audience.
                    audience::mark_dashboard_open(core, stream.id, interval).await?;
                } else {
                    // A guest is whatever client ID the request names, so only so
                    // many per address count; the rest watch without being counted.
                    let counted = match user_id {
                        Some(_) => true,
                        None => audience::admit_guest(core, stream.id, &ctx.client().ip, &key, interval).await?,
                    };
                    if counted {
                        let since_last = audience::heartbeat(core, stream.id, &key, interval).await?;
                        let props =
                            serde_json::json!({ "channel_id": stream.channel_id, "viewer": key.analytics_name() });
                        let event = match since_last {
                            None => AnalyticsEvent::new("live.view").value(1.0),
                            Some(seconds) => AnalyticsEvent::new("live.watch").value(seconds as f64),
                        };
                        core.analytics
                            .track(
                                event
                                    .user(user_id)
                                    .subject("live_stream", stream.id)
                                    .owner(stream.owner_id)
                                    .props(props),
                            )
                            .await;
                    }
                }
            }
            // Playback tokens are short-lived: every heartbeat brings the next one,
            // as long as the viewer may still watch (which was just checked).
            let playable = self.cfg.media_server_configured()
                && (stream.is_live() || (viewer.is(stream.owner_id) && stream.encoder_connected && !stream.is_ended()));
            ok(ReportWatchingResponse {
                viewer_count: if stream.is_live() { stream.viewer_count } else { 0 },
                like_count: stream.like_count,
                status: stream.status.into(),
                report_interval_seconds: interval as i32,
                playback: playable.then(|| self.playback_for(&stream, user_id, session)).flatten().into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_viewers(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListViewersRequest>,
    ) -> ServiceResult<ListViewersResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let stream = streams::find_watchable(core, &viewer, request.stream_id).await?;
            access::require_moderator(core, stream.channel_id, stream.owner_id, user_id).await?;
            core.cache.rate_limit(&format!("live-lists:{user_id}"), 60, Duration::from_secs(60)).await?;
            let page = Page::from_view(request.page.as_option(), 100);
            let skip: usize = page.decode()?.unwrap_or(0);
            // People in a block relation with the caller (a moderator) stay hidden from them here too.
            let watching: Vec<(Uuid, Time)> =
                audience::watching_users(core, stream.id, self.cfg.report_interval_seconds)
                    .await?
                    .into_iter()
                    .filter(|(id, _)| !viewer.hides(*id))
                    .collect();
            let total_signed_in = watching.len();
            let mut slice: Vec<(usize, Uuid, Time)> = watching
                .into_iter()
                .enumerate()
                .skip(skip)
                .take(page.fetch() as usize)
                .map(|(i, (id, at))| (i, id, at))
                .collect();
            let info = page.finish(&mut slice, |(position, _, _)| position + 1);
            let ids: Vec<Uuid> = slice.iter().map(|(_, id, _)| *id).collect();
            let users = tokuly_identity::summaries(core, &ids).await?;
            let mut viewers = Vec::with_capacity(slice.len());
            for (_, id, last_seen_at) in slice {
                let standing = access::standing(core, stream.channel_id, stream.owner_id, id).await?;
                let subscribed = channels::is_subscribed(core, id, stream.channel_id).await?;
                viewers.push(tokuly_proto::live::Viewer {
                    user: Some(tokuly_identity::users::pick(&users, id)).into(),
                    role: chat::role_from_standing(standing, subscribed).into(),
                    last_seen_at: ts(last_seen_at),
                    ..Default::default()
                });
            }
            ok(ListViewersResponse {
                viewers,
                page: info.into(),
                viewer_count: stream.viewer_count.max(total_signed_in as i64),
                ..Default::default()
            })
        })
        .await
    }

    async fn raid_stream(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RaidStreamRequest>,
    ) -> ServiceResult<RaidStreamResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let stream = streams::find_owned(core, user_id, request.stream_id).await?;
            ensure!(stream.is_live(), precondition, "STREAM_NOT_LIVE", "only a stream on air can raid");
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let target = streams::find_ref(core, request.target_stream_id).await?;
            // The audience must be able to follow: the target is open to everyone.
            let target = target
                .filter(|target| {
                    access::can_watch(&viewer, &target.facts()) && target.open_to_everyone() && !target.suspended
                })
                .ok_or_else(|| AppError::not_found("RAID_TARGET_NOT_FOUND", "the stream to raid was not found"))?;
            ensure!(target.channel_id != stream.channel_id, invalid, "RAID_SELF", "a channel cannot raid itself");
            ensure!(target.is_live(), precondition, "RAID_TARGET_NOT_LIVE", "the stream to raid is not on air");
            core.cache.rate_limit(&format!("live-raid:{user_id}"), 5, Duration::from_secs(3600)).await?;

            let viewers = stream.viewer_count;
            let raiders = channels::summaries(core, &[stream.channel_id]).await?.remove(&stream.channel_id);
            let target_summary = streams::summary(core, &target).await?;
            // Ending the stream is what makes a raid happen once: of two
            // requests at the same moment only one ends it, and only that
            // one tells the audience and the target.
            let ended_now =
                self.finish_stream(stream.id, EndReason::END_REASON_RAID, Some(target.id), Some(user_id)).await?;
            ensure!(ended_now.is_some(), precondition, "STREAM_NOT_LIVE", "only a stream on air can raid");
            let raid = Raid {
                stream_id: stream.id.to_string(),
                target: Some(target_summary.clone()).into(),
                viewer_count: viewers,
                ..Default::default()
            };
            self.push_to_stream(stream.id, Event::Raid(Box::new(raid))).await;

            let notice = ChatNotice {
                kind: ChatNoticeKind::CHAT_NOTICE_KIND_RAID.into(),
                channel: raiders.clone().into(),
                viewer_count: viewers,
                ..Default::default()
            };
            let mut tx = core.db.begin().await?;
            let line = chat::insert_in(
                &mut tx,
                NewMessage {
                    stream_id: target.id,
                    user_id: None,
                    kind: chat::KIND_NOTICE,
                    role: ChatRole::CHAT_ROLE_UNSPECIFIED,
                    text: "",
                    stamp_id: None,
                    gift: None,
                    notice: Some(&notice),
                },
            )
            .await?;
            tx.commit().await?;
            self.push_chat_message(&target, &line).await;
            let alert = Alert {
                kind: AlertKind::ALERT_KIND_RAID.into(),
                name: raiders.as_ref().map(|channel| channel.name.clone()).unwrap_or_default(),
                avatar: raiders.as_ref().map(|channel| channel.icon.clone()).unwrap_or_default(),
                viewer_count: viewers,
                ..Default::default()
            };
            self.push_to_overlays(
                target.channel_id,
                &[OverlayKind::OVERLAY_KIND_ALERT_BOX],
                Event::Alert(Box::new(alert)),
            )
            .await;
            let ended = streams::find_by_id(core, stream.id).await?.ok_or_else(streams::not_found)?;
            ok(RaidStreamResponse {
                stream: Some(self.own_stream_proto(ctx.auth()?, &ended).await?).into(),
                target: Some(target_summary).into(),
                viewer_count: viewers,
                ..Default::default()
            })
        })
        .await
    }
}
