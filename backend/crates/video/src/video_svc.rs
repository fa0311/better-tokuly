//! `VideoService`: the studio side (create, edit, publish, subtitles) and
//! the watch page (get, rate).
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::{ids::Ref, prelude::*};
use tokuly_media::assets;
use tokuly_proto::{common::Visibility, media::AssetPurpose, video::*};

use crate::{
    Video,
    access::{self, Viewer, watchable},
    analytics, channels, games, party, playlists,
    subtitles::{self, SubtitleEdit},
    validate,
    videos::{self, NewVideo, VideoRow, video_columns},
};

/// Most videos one `BatchGetVideos` call may ask for.
const MAX_BATCH: usize = 100;

fn video_not_found() -> AppError {
    AppError::not_found("VIDEO_NOT_FOUND", "video not found")
}

/// A visibility named in an update: it must be one of the five values.
fn explicit_visibility(value: buffa::EnumValue<Visibility>) -> AppResult<i32> {
    match value.as_known() {
        Some(Visibility::VISIBILITY_UNSPECIFIED) | None => {
            Err(AppError::invalid("VISIBILITY_INVALID", "unknown visibility"))
        }
        Some(Visibility::VISIBILITY_FOLLOWERS | Visibility::VISIBILITY_CLOSE_FRIENDS | Visibility::VISIBILITY_CIRCLE) => {
            Err(AppError::invalid("VISIBILITY_INVALID", "this audience is not available here yet"))
        }
        Some(known) => Ok(known as i32),
    }
}

impl Video {
    async fn video_response(&self, user_id: Uuid, video_id: Uuid) -> AppResult<tokuly_proto::video::Video> {
        let row = videos::find_by_id(&self.core, video_id).await?.ok_or_else(video_not_found)?;
        let viewer = Viewer { user_id: Some(user_id), ..Default::default() };
        videos::to_proto(&self.core, &viewer, &row).await
    }
}

impl VideoService for Video {
    async fn create_video(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateVideoRequest>,
    ) -> ServiceResult<CreateVideoResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let channel = match req.channel_id.trim() {
                "" => channels::default_for(core, user_id).await?.ok_or_else(|| {
                    AppError::precondition("CHANNEL_REQUIRED", "create a channel before uploading videos")
                })?,
                reference => channels::require_owned_ref(core, user_id, reference).await?,
            };
            // The upload must be the caller's own and meant to become a video.
            let asset_id = parse_id(&req.asset_id)?;
            assets::require(core, asset_id, user_id, &[AssetPurpose::ASSET_PURPOSE_VIDEO], false).await?;
            let visibility = req
                .visibility
                .as_known()
                .ok_or_else(|| AppError::invalid("VISIBILITY_INVALID", "unknown visibility"))?;
            let new = NewVideo {
                title: req.title,
                description: req.description,
                tags: req.tags,
                category: validate::category(req.category)?,
                game: req.game,
                visibility,
                allow_link: req.allow_link,
                age_restricted: req.age_restricted,
                comments_enabled: req.comments_enabled.unwrap_or(true),
                notify_subscribers: req.notify_subscribers.unwrap_or(true),
                thumbnail_asset_id: parse_opt_id(&req.thumbnail_asset_id)?,
                chapters: req.chapters,
                ..NewVideo::new(channel.id, asset_id, String::new())
            };
            core.cache.rate_limit(&format!("video-create:{user_id}"), 30, Duration::from_secs(3600)).await?;
            let video_id = videos::create_from_asset(core, new).await?;
            ok(CreateVideoResponse {
                video: self.video_response(user_id, video_id).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_video(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateVideoRequest>,
    ) -> ServiceResult<UpdateVideoResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let video = videos::find_owned(core, user_id, &req.video_id).await?;

            let title = req.title.as_deref().map(validate::title).transpose()?;
            let description = req.description.as_deref().map(validate::description).transpose()?;
            let tags = req.tags.into_option().map(|list| validate::tags(&list.tags)).transpose()?;
            let category = req.category.map(validate::category).transpose()?;
            let game = req.game.as_deref().map(validate::game).transpose()?;
            let visibility = req.visibility.map(explicit_visibility).transpose()?;
            let duration = video.is_ready().then_some(video.duration_seconds);
            let chapters = req.chapters.into_option().map(|list| validate::chapters(&list.chapters, duration)).transpose()?;
            // `Some(None)`: back to the poster frame.
            let thumbnail = match req.thumbnail_asset_id.as_deref().map(str::trim) {
                None => None,
                Some("") => Some(None),
                Some(id) => {
                    let asset_id = parse_id(id)?;
                    let image = assets::require_image_json(core, asset_id, user_id, &[AssetPurpose::ASSET_PURPOSE_THUMBNAIL]).await?;
                    Some(Some((asset_id, image)))
                }
            };

            let mut tx = core.db.begin().await?;
            // What the change starts from is the row as it is under the lock,
            // not as it was read above: the scheduler may have published it
            // in between.
            let video = videos::find_in(&mut tx, video.id).await?.ok_or_else(video_not_found)?;
            sqlx::query(
                "UPDATE videos SET
                     title = COALESCE($2, title), description = COALESCE($3, description), tags = COALESCE($4, tags),
                     category = COALESCE($5, category), game = COALESCE($6, game), visibility = COALESCE($7, visibility),
                     chapters = COALESCE($8, chapters), age_restricted = COALESCE($9, age_restricted),
                     comments_enabled = COALESCE($10, comments_enabled), notify_subscribers = COALESCE($11, notify_subscribers),
                     allow_link = COALESCE($12, allow_link) AND COALESCE($7, visibility) = 4
                 WHERE id = $1",
            )
            .bind(video.id)
            .bind(&title)
            .bind(&description)
            .bind(&tags)
            .bind(category)
            .bind(&game)
            .bind(visibility)
            .bind(chapters.as_deref().map(videos::chapters_json).transpose()?)
            .bind(req.age_restricted)
            .bind(req.comments_enabled)
            .bind(req.notify_subscribers)
            .bind(req.allow_link)
            .execute(&mut *tx)
            .await?;
            let thumbnail_changed = thumbnail.is_some();
            if let Some(change) = thumbnail {
                let holder = videos::holder(video.id);
                if let Some(previous) = video.thumbnail_asset_id {
                    assets::detach(&mut *tx, previous, &holder).await?;
                }
                match change {
                    Some((asset_id, image)) => {
                        assets::attach(&mut *tx, asset_id, &holder).await?;
                        sqlx::query("UPDATE videos SET thumbnail = $2, thumbnail_asset_id = $3 WHERE id = $1")
                            .bind(video.id)
                            .bind(image)
                            .bind(asset_id)
                            .execute(&mut *tx)
                            .await?;
                    }
                    None => {
                        sqlx::query("UPDATE videos SET thumbnail = playback -> 'poster', thumbnail_asset_id = NULL WHERE id = $1")
                            .bind(video.id)
                            .execute(&mut *tx)
                            .await?;
                    }
                }
            }
            let row = videos::find_in(&mut tx, video.id).await?.ok_or_else(video_not_found)?;
            let row = videos::after_visible_change_in(&mut tx, video.is_public(), row, false).await?;
            // A new picture of a video that stays public: the playlists
            // showing it follow (a change of audience did that already).
            if thumbnail_changed && video.is_public() && row.is_public() {
                playlists::sync_search_of_video_in(&mut tx, row.id).await?;
            }
            videos::announce_visibility_in(&mut tx, &video, &row).await?;
            tx.commit().await?;
            // Nobody but the owner can follow a video that went private.
            if row.is_published() && row.visibility == access::PRIVATE && video.visibility != access::PRIVATE {
                party::end_for_video(core, row.id).await?;
            }
            ok(UpdateVideoResponse { video: self.video_response(user_id, row.id).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn publish_video(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, PublishVideoRequest>,
    ) -> ServiceResult<PublishVideoResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let video = videos::find_owned(core, user_id, &req.video_id).await?;
            let visibility = req.visibility.map(explicit_visibility).transpose()?;
            let publish_at = req.publish_at.as_option().map(from_ts);

            let mut tx = core.db.begin().await?;
            // Locked before anything is decided from it (see `update_video`).
            let video = videos::find_in(&mut tx, video.id).await?.ok_or_else(video_not_found)?;
            if let Some(visibility) = visibility {
                sqlx::query("UPDATE videos SET visibility = $2, allow_link = allow_link AND $2 = 4 WHERE id = $1")
                    .bind(video.id)
                    .bind(visibility)
                    .execute(&mut *tx)
                    .await?;
            }
            let row = videos::find_in(&mut tx, video.id).await?.ok_or_else(video_not_found)?;
            if row.is_published() {
                // Already live: only the visibility may have changed.
                let row = videos::after_visible_change_in(&mut tx, video.is_public(), row, false).await?;
                videos::announce_visibility_in(&mut tx, &video, &row).await?;
            } else {
                videos::request_publish_in(&mut tx, &row, publish_at).await?;
            }
            tx.commit().await?;
            ok(PublishVideoResponse {
                video: self.video_response(user_id, video.id).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn unpublish_video(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnpublishVideoRequest>,
    ) -> ServiceResult<UnpublishVideoResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let video = videos::find_owned(core, user_id, request.video_id).await?;
            let mut tx = core.db.begin().await?;
            let locked = videos::find_in(&mut tx, video.id).await?.ok_or_else(video_not_found)?;
            videos::unpublish_in(&mut tx, &locked).await?;
            tx.commit().await?;
            if locked.is_published() {
                party::end_for_video(core, video.id).await?;
            }
            ok(UnpublishVideoResponse {
                video: self.video_response(user_id, video.id).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_video(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteVideoRequest>,
    ) -> ServiceResult<DeleteVideoResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let video = videos::find_owned(&self.core, user_id, request.video_id).await?;
            videos::delete(&self.core, &video, Some(user_id), false).await?;
            ok(DeleteVideoResponse::default())
        })
        .await
    }

    async fn get_video(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetVideoRequest>,
    ) -> ServiceResult<GetVideoResponse> {
        handle(async {
            let viewer = Viewer::load(&self.core, ctx.maybe_user_id()?).await?;
            let video = videos::find_watchable(&self.core, &viewer, request.video_id).await?;
            ok(GetVideoResponse {
                video: videos::to_proto(&self.core, &viewer, &video).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn batch_get_videos(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, BatchGetVideosRequest>,
    ) -> ServiceResult<BatchGetVideosResponse> {
        handle(async {
            ensure!(
                request.video_ids.len() <= MAX_BATCH,
                invalid,
                "TOO_MANY_IDS",
                "at most {MAX_BATCH} videos per request"
            );
            let viewer = Viewer::load(&self.core, ctx.maybe_user_id()?).await?;
            // Malformed references name no video and are skipped like unknown ones.
            let references: Vec<Ref> = request.video_ids.iter().filter_map(|id| Ref::parse(id.trim()).ok()).collect();
            let ids: Vec<Uuid> =
                references.iter().filter_map(|r| if let Ref::Id(id) = r { Some(*id) } else { None }).collect();
            let short_ids: Vec<String> = references
                .iter()
                .filter_map(|r| if let Ref::Short(id) = r { Some(id.clone()) } else { None })
                .collect();
            let mut rows: Vec<VideoRow> = sqlx::query_as(concat!(
                "SELECT ",
                video_columns!(),
                " FROM videos v WHERE (v.id = ANY($1) OR v.short_id = ANY($2)) AND ",
                watchable!(3, 4, 5)
            ))
            .bind(&ids)
            .bind(&short_ids)
            .bind(viewer.user_id)
            .bind(&viewer.friends)
            .bind(&viewer.blocked)
            .fetch_all(&self.core.db)
            .await?;
            let order = |row: &VideoRow| {
                references
                    .iter()
                    .position(|reference| match reference {
                        Ref::Id(id) => *id == row.id,
                        Ref::Short(short_id) => *short_id == row.short_id,
                    })
                    .unwrap_or(usize::MAX)
            };
            rows.sort_by_key(order);
            ok(BatchGetVideosResponse {
                videos: videos::summaries(&self.core, &viewer, &rows).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn list_studio_videos(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListStudioVideosRequest>,
    ) -> ServiceResult<ListStudioVideosResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let channel_id = match request.channel_id.trim() {
                "" => None,
                reference => Some(channels::require_owned_ref(&self.core, user_id, reference).await?.id),
            };
            ensure!(request.status.as_known().is_some(), invalid, "STATUS_INVALID", "unknown video status");
            let page = Page::from_view(request.page.as_option(), 100);
            let before: Option<Uuid> = page.decode()?;
            let mut rows: Vec<VideoRow> = sqlx::query_as(concat!(
                "SELECT ",
                video_columns!(),
                " FROM videos v
                  WHERE v.owner_id = $1 AND ($2::uuid IS NULL OR v.channel_id = $2) AND ($3 = 0 OR v.status = $3)
                    AND ($4::uuid IS NULL OR v.id < $4)
                  ORDER BY v.id DESC
                  LIMIT $5"
            ))
            .bind(user_id)
            .bind(channel_id)
            .bind(request.status.to_i32())
            .bind(before)
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
            let info = page.finish(&mut rows, |row| row.id);
            let viewer = Viewer { user_id: Some(user_id), ..Default::default() };
            ok(ListStudioVideosResponse {
                videos: videos::summaries(&self.core, &viewer, &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn rate_video(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RateVideoRequest>,
    ) -> ServiceResult<RateVideoResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let rating = match request.rating.as_known() {
                Some(Rating::RATING_LIKE) => Some(videos::LIKE),
                Some(Rating::RATING_DISLIKE) => Some(videos::DISLIKE),
                Some(Rating::RATING_NONE) => None,
                _ => bail!(invalid, "RATING_INVALID", "rating must be LIKE, DISLIKE or NONE"),
            };
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let video = videos::find_watchable(core, &viewer, request.video_id).await?;
            ensure!(video.is_published() || viewer.is(video.owner_id), not_found, "VIDEO_NOT_FOUND", "video not found");
            core.cache.rate_limit(&format!("video-rate:{user_id}"), 60, Duration::from_secs(60)).await?;
            let rated = videos::rate(core, user_id, &video, rating).await?;
            if let Some(rating) = rating.filter(|rating| rated.previous != Some(*rating) && user_id != video.owner_id) {
                analytics::track_rating(core, &video, user_id, rating == videos::LIKE).await;
            }
            ok(RateVideoResponse {
                rating: rating.unwrap_or(Rating::RATING_NONE as i32).into(),
                like_count: rated.like_count,
                ..Default::default()
            })
        })
        .await
    }

    async fn suggest_games(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SuggestGamesRequest>,
    ) -> ServiceResult<SuggestGamesResponse> {
        handle(async {
            let limiter = ctx.maybe_user_id()?.map(|id| id.to_string()).unwrap_or_else(|| ctx.client().ip);
            self.core.cache.rate_limit(&format!("video-games:{limiter}"), 240, Duration::from_secs(60)).await?;
            ensure!(
                request.query.chars().count() <= validate::MAX_GAME,
                invalid,
                "QUERY_TOO_LONG",
                "the query is too long"
            );
            let limit = if request.limit <= 0 { 10 } else { request.limit.min(20) } as i64;
            ok(SuggestGamesResponse {
                games: games::suggest(&self.core, request.query, limit).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn add_subtitle(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AddSubtitleRequest>,
    ) -> ServiceResult<AddSubtitleResponse> {
        handle(async {
            use add_subtitle_request::Source;
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let video = videos::find_owned(&self.core, user_id, &req.video_id).await?;
            let source = match req.source {
                Some(Source::AssetId(id)) => subtitles::Source::Asset(parse_id(&id)?),
                Some(Source::Content(content)) => subtitles::Source::Content(content),
                None => bail!(invalid, "SUBTITLE_SOURCE_REQUIRED", "an uploaded file or its text is required"),
            };
            self.core.cache.rate_limit(&format!("video-subtitle:{user_id}"), 120, Duration::from_secs(3600)).await?;
            let subtitle =
                subtitles::add(&self.core, &video, source, &req.language, &req.label, req.is_default).await?;
            ok(AddSubtitleResponse { subtitle: subtitle.into(), ..Default::default() })
        })
        .await
    }

    async fn update_subtitle(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateSubtitleRequest>,
    ) -> ServiceResult<UpdateSubtitleResponse> {
        handle(async {
            use update_subtitle_request::Source;
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let track = subtitles::find_owned(&self.core, user_id, parse_id(&req.subtitle_id)?).await?;
            let source = match req.source {
                Some(Source::AssetId(id)) => Some(subtitles::Source::Asset(parse_id(&id)?)),
                Some(Source::Content(content)) => Some(subtitles::Source::Content(content)),
                None => None,
            };
            if source.is_some() {
                self.core
                    .cache
                    .rate_limit(&format!("video-subtitle:{user_id}"), 120, Duration::from_secs(3600))
                    .await?;
            }
            let edit = SubtitleEdit { language: req.language, label: req.label, is_default: req.is_default, source };
            let subtitle = subtitles::update(&self.core, user_id, &track, edit).await?;
            ok(UpdateSubtitleResponse { subtitle: subtitle.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_subtitle(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteSubtitleRequest>,
    ) -> ServiceResult<DeleteSubtitleResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let track = subtitles::find_owned(&self.core, user_id, parse_id(request.subtitle_id)?).await?;
            subtitles::delete(&self.core, &track).await?;
            ok(DeleteSubtitleResponse::default())
        })
        .await
    }

    async fn list_subtitles(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListSubtitlesRequest>,
    ) -> ServiceResult<ListSubtitlesResponse> {
        handle(async {
            let viewer = Viewer::load(&self.core, ctx.maybe_user_id()?).await?;
            let video = videos::find_watchable(&self.core, &viewer, request.video_id).await?;
            ok(ListSubtitlesResponse { subtitles: subtitles::list(&self.core, video.id).await?, ..Default::default() })
        })
        .await
    }
}
