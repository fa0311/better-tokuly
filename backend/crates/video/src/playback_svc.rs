//! `PlaybackService`: playback sessions and the watch history built from them.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::video::*;

use crate::{
    Video,
    access::{self, Viewer, watchable},
    playback,
    videos::{self, VideoRow, video_columns},
};

#[derive(sqlx::FromRow)]
struct HistoryRow {
    #[sqlx(flatten)]
    video: VideoRow,
    last_watched_at: Time,
    watched_seconds: f64,
}

impl Video {
    /// Seconds a session of this video must watch before it counts as a view.
    fn min_view_seconds(&self, duration: f64) -> f64 {
        playback::min_view_seconds(self.cfg.min_view_seconds, duration)
    }

    /// Longest stretch between two reports that still counts as watching.
    fn max_report_gap(&self) -> f64 {
        f64::from(self.cfg.report_interval_seconds) * 3.0
    }
}

impl PlaybackService for Video {
    async fn start_playback(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, StartPlaybackRequest>,
    ) -> ServiceResult<StartPlaybackResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.maybe_user_id()?;
            let client = ctx.client();
            let viewer = Viewer::load(core, user_id).await?;
            let video = videos::find_watchable(core, &viewer, request.video_id).await?;
            ensure!(video.is_ready(), precondition, "PLAYBACK_BLOCKED", "the video is not ready to play");
            let blocked = access::age_block(core, &viewer, video.owner_id, video.age_restricted).await?;
            ensure!(blocked.is_none(), denied, "PLAYBACK_BLOCKED", "this video is age-restricted");
            ensure!(request.source.as_known().is_some(), invalid, "SOURCE_INVALID", "unknown traffic source");
            let limiter = playback::viewer_key(core, user_id, &client.ip);
            core.cache.rate_limit(&format!("video-play:{limiter}"), 120, Duration::from_secs(60)).await?;

            let resume = match user_id {
                Some(user_id) => {
                    playback::touch_history(core, user_id, video.id).await?;
                    playback::resume_position(core, user_id, video.id).await?
                }
                None => 0.0,
            };
            let session_id =
                playback::start(core, &video, user_id, &client.ip, request.source.to_i32(), request.position_seconds)
                    .await?;
            ok(StartPlaybackResponse {
                session_id,
                resume_position_seconds: resume,
                min_view_seconds: self.min_view_seconds(video.duration_seconds),
                report_interval_seconds: self.cfg.report_interval_seconds as i32,
                view_count: video.view_count,
                ..Default::default()
            })
        })
        .await
    }

    async fn report_playback(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ReportPlaybackRequest>,
    ) -> ServiceResult<ReportPlaybackResponse> {
        handle(async {
            let core = &self.core;
            let caller = ctx.maybe_user_id()?;
            let gone =
                || AppError::not_found("PLAYBACK_SESSION_NOT_FOUND", "the playback session expired; start a new one");
            let session_id = request.session_id;
            let mut session = playback::load(core, session_id).await?.ok_or_else(gone)?;
            // Players report every few seconds; anything much faster is not a player.
            core.cache.rate_limit(&format!("video-report:{session_id}"), 120, Duration::from_secs(60)).await?;
            // A session belongs to whoever opened it.
            ensure!(
                session.user_id.is_none() || session.user_id == caller,
                not_found,
                "PLAYBACK_SESSION_NOT_FOUND",
                "playback session not found"
            );
            // The video may have been deleted while it played: the session ends with it.
            let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM videos WHERE id = $1)")
                .bind(session.video_id)
                .fetch_one(&core.db)
                .await?;
            if !exists {
                core.cache.del(&playback::session_key(session_id)).await?;
                return Err(gone());
            }

            let playing = request.playing && !request.ended;
            let progress = playback::advance(
                &mut session,
                now().timestamp_millis(),
                request.position_seconds,
                playing,
                self.max_report_gap(),
            );
            let dedupe = Duration::from_secs(self.cfg.view_dedupe_seconds);
            let min_seconds = self.min_view_seconds(session.duration);
            playback::count_view(core, &mut session, session_id, min_seconds, dedupe).await?;
            playback::save(core, session_id, &session).await?;
            playback::track_progress(core, &session, session_id, progress).await;
            if let Some(user_id) = session.user_id {
                playback::record_history(
                    core,
                    user_id,
                    &session,
                    session.last_position,
                    progress.watched,
                    request.ended,
                )
                .await?;
            }
            let view_count: Option<i64> = sqlx::query_scalar("SELECT view_count FROM videos WHERE id = $1")
                .bind(session.video_id)
                .fetch_optional(&core.db)
                .await?;
            let view_count = view_count.ok_or_else(gone)?;
            ok(ReportPlaybackResponse { view_counted: session.counted, view_count, ..Default::default() })
        })
        .await
    }

    async fn list_watch_history(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListWatchHistoryRequest>,
    ) -> ServiceResult<ListWatchHistoryResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let page = Page::from_view(request.page.as_option(), 100);
            let before: Option<(Time, Uuid)> = page.decode()?;
            // Videos that were since made private (or whose owner is hidden
            // from the viewer) stay in the table but are not shown.
            let mut rows: Vec<HistoryRow> = viewer
                .query_as(concat!(
                    "SELECT ",
                    video_columns!(),
                    ", h.last_watched_at, h.watched_seconds
                     FROM watch_history h JOIN videos v ON v.id = h.video_id
                     WHERE h.user_id = $1 AND ",
                    watchable!(),
                    "  AND ($7::timestamptz IS NULL OR (h.last_watched_at, h.video_id) < ($7, $8))
                     ORDER BY h.last_watched_at DESC, h.video_id DESC
                     LIMIT $9"
                ))
                .bind(before.map(|(at, _)| at))
                .bind(before.map(|(_, id)| id))
                .bind(page.fetch())
                .fetch_all(&core.db)
                .await?;
            let info = page.finish(&mut rows, |row| (row.last_watched_at, row.video.id));
            viewer.drop_newly_locked(core, &mut rows, |row| row.video.lock_facts()).await?;
            let videos: Vec<VideoRow> = rows.iter().map(|row| row.video.clone()).collect();
            let summaries = videos::summaries(core, &viewer, &videos).await?;
            let items = rows
                .iter()
                .zip(summaries)
                .map(|(row, video)| WatchHistoryItem {
                    video: video.into(),
                    last_watched_at: ts(row.last_watched_at),
                    watched_seconds: row.watched_seconds,
                    ..Default::default()
                })
                .collect();
            ok(ListWatchHistoryResponse {
                items,
                page: info.into(),
                paused: playback::history_paused(core, user_id).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn remove_watch_history_item(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveWatchHistoryItemRequest>,
    ) -> ServiceResult<RemoveWatchHistoryItemResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            // Whatever the video's state: one can always forget having watched something.
            if let Some(video) = videos::find_ref(&self.core, request.video_id).await? {
                sqlx::query("DELETE FROM watch_history WHERE user_id = $1 AND video_id = $2")
                    .bind(user_id)
                    .bind(video.id)
                    .execute(&self.core.db)
                    .await?;
            }
            ok(RemoveWatchHistoryItemResponse::default())
        })
        .await
    }

    async fn clear_watch_history(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ClearWatchHistoryRequest>,
    ) -> ServiceResult<ClearWatchHistoryResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            sqlx::query("DELETE FROM watch_history WHERE user_id = $1").bind(user_id).execute(&self.core.db).await?;
            ok(ClearWatchHistoryResponse::default())
        })
        .await
    }

    async fn get_watch_history_settings(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetWatchHistorySettingsRequest>,
    ) -> ServiceResult<GetWatchHistorySettingsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            ok(GetWatchHistorySettingsResponse {
                paused: playback::history_paused(&self.core, user_id).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn set_watch_history_paused(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetWatchHistoryPausedRequest>,
    ) -> ServiceResult<SetWatchHistoryPausedResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            playback::set_history_paused(&self.core, user_id, request.paused).await?;
            ok(SetWatchHistoryPausedResponse { paused: request.paused, ..Default::default() })
        })
        .await
    }
}
