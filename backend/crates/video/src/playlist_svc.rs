//! `PlaylistService`: playlists and the two built-in lists.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::video::*;

use crate::{
    Video,
    access::{self, Viewer, watchable},
    channels, locks,
    playlists::{self, KIND_CUSTOM, KIND_LIKED, KIND_WATCH_LATER, PlaylistRow, playlist_columns},
    validate,
    videos::{self, VideoRow, video_columns},
};

#[derive(sqlx::FromRow)]
struct ItemRow {
    #[sqlx(flatten)]
    video: VideoRow,
    position: i64,
    added_at: Time,
}

fn built_in() -> AppError {
    AppError::precondition("PLAYLIST_BUILT_IN", "built-in playlists cannot be edited or deleted")
}

fn read_only() -> AppError {
    AppError::precondition("PLAYLIST_READ_ONLY", "liked videos change by liking and unliking videos")
}

impl Video {
    async fn playlist_response(&self, viewer: &Viewer, playlist_id: Uuid) -> AppResult<Playlist> {
        let playlist = playlists::find_by_id(&self.core, playlist_id)
            .await?
            .or_not_found("PLAYLIST_NOT_FOUND", "playlist not found")?;
        playlists::to_proto(&self.core, viewer, &playlist).await
    }

    /// The channel a playlist is to be published on: one of the caller's.
    async fn playlist_channel(&self, user_id: Uuid, reference: &str) -> AppResult<Option<Uuid>> {
        match reference.trim() {
            "" => Ok(None),
            reference => Ok(Some(channels::require_owned_ref(&self.core, user_id, reference).await?.id)),
        }
    }
}

impl PlaylistService for Video {
    async fn create_playlist(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreatePlaylistRequest>,
    ) -> ServiceResult<CreatePlaylistResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let title = validate::playlist_title(request.title)?;
            let description = validate::playlist_description(request.description)?;
            let visibility = validate::playlist_visibility(request.visibility)?;
            let channel_id = self.playlist_channel(user_id, request.channel_id).await?;
            let first_video = match request.video_id.trim() {
                "" => None,
                reference => Some(videos::find_watchable(core, &viewer, reference).await?.id),
            };
            core.cache.rate_limit(&format!("video-playlist-create:{user_id}"), 30, Duration::from_secs(3600)).await?;
            let owner_locked = access::is_locked_now(core, user_id).await?;

            let mut tx = core.db.begin().await?;
            // One at a time per account, so the limit holds.
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                .bind(format!("video:playlists:{}", user_id.simple()))
                .execute(&mut *tx)
                .await?;
            let owned: i64 = sqlx::query_scalar("SELECT count(*) FROM playlists WHERE owner_id = $1 AND kind = 1")
                .bind(user_id)
                .fetch_one(&mut *tx)
                .await?;
            let max = self.cfg.max_playlists;
            ensure!(owned < max, precondition, "PLAYLIST_LIMIT_REACHED", "an account can have at most {max} playlists");
            let playlist_id = new_id();
            sqlx::query(
                "INSERT INTO playlists (id, short_id, owner_id, channel_id, kind, title, description, visibility, owner_locked)
                 VALUES ($1, $2, $3, $4, 1, $5, $6, $7, $8)",
            )
            .bind(playlist_id)
            .bind(tokuly_core::ids::new_short_id())
            .bind(user_id)
            .bind(channel_id)
            .bind(&title)
            .bind(&description)
            .bind(visibility)
            .bind(owner_locked)
            .execute(&mut *tx)
            .await?;
            if let Some(video_id) = first_video {
                playlists::add_item_in(&mut tx, playlist_id, video_id).await?;
            }
            playlists::sync_search_in(&mut tx, false, playlist_id).await?;
            tx.commit().await?;
            // Locked or unlocked meanwhile: see `videos::create_from_asset`.
            if access::is_locked_now(core, user_id).await? != owner_locked {
                locks::sync_owner(core, user_id).await?;
            }
            ok(CreatePlaylistResponse {
                playlist: self.playlist_response(&viewer, playlist_id).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_playlist(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdatePlaylistRequest>,
    ) -> ServiceResult<UpdatePlaylistResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let playlist = playlists::resolve_owned(core, user_id, &req.playlist_id).await?;
            if playlist.is_builtin() {
                return Err(built_in());
            }
            let title = req.title.as_deref().map(validate::playlist_title).transpose()?;
            let description = req.description.as_deref().map(validate::playlist_description).transpose()?;
            let visibility = match req.visibility {
                // Naming no visibility in an update is not "private".
                Some(value) if value.as_known() == Some(tokuly_proto::common::Visibility::VISIBILITY_UNSPECIFIED) => {
                    bail!(invalid, "VISIBILITY_INVALID", "playlists are public, unlisted or private")
                }
                Some(value) => Some(validate::playlist_visibility(value)?),
                None => None,
            };
            // `Some(None)`: make it a personal playlist.
            let channel_id = match req.channel_id.as_deref() {
                Some(reference) => Some(self.playlist_channel(user_id, reference).await?),
                None => None,
            };

            let mut tx = core.db.begin().await?;
            sqlx::query(
                "UPDATE playlists SET title = COALESCE($2, title), description = COALESCE($3, description),
                        visibility = COALESCE($4, visibility),
                        channel_id = CASE WHEN $5 THEN $6 ELSE channel_id END, updated_at = now()
                 WHERE id = $1",
            )
            .bind(playlist.id)
            .bind(&title)
            .bind(&description)
            .bind(visibility)
            .bind(channel_id.is_some())
            .bind(channel_id.flatten())
            .execute(&mut *tx)
            .await?;
            playlists::sync_search_in(&mut tx, playlist.is_public(), playlist.id).await?;
            tx.commit().await?;
            let viewer = Viewer { user_id: Some(user_id), ..Default::default() };
            ok(UpdatePlaylistResponse {
                playlist: self.playlist_response(&viewer, playlist.id).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_playlist(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeletePlaylistRequest>,
    ) -> ServiceResult<DeletePlaylistResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let playlist = playlists::resolve_owned(&self.core, user_id, request.playlist_id).await?;
            if playlist.is_builtin() {
                return Err(built_in());
            }
            let mut tx = self.core.db.begin().await?;
            playlists::delete_in(&mut tx, playlist.id, user_id, Some(user_id)).await?;
            tx.commit().await?;
            ok(DeletePlaylistResponse::default())
        })
        .await
    }

    async fn get_playlist(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetPlaylistRequest>,
    ) -> ServiceResult<GetPlaylistResponse> {
        handle(async {
            let viewer = Viewer::load(&self.core, ctx.maybe_user_id()?).await?;
            let playlist = playlists::resolve(&self.core, &viewer, request.playlist_id).await?;
            ok(GetPlaylistResponse {
                playlist: playlists::to_proto(&self.core, &viewer, &playlist).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_my_playlists(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListMyPlaylistsRequest>,
    ) -> ServiceResult<ListMyPlaylistsResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let contains = match request.contains_video_id.trim() {
                "" => None,
                reference => Some(
                    videos::find_ref(core, reference).await?.or_not_found("VIDEO_NOT_FOUND", "video not found")?.id,
                ),
            };
            // The built-in lists exist from the first time they are looked at.
            let mut tx = core.db.begin().await?;
            for kind in [KIND_WATCH_LATER, KIND_LIKED] {
                playlists::builtin_in(&mut tx, user_id, kind).await?;
            }
            tx.commit().await?;
            // Built-in lists first (watch later, then liked), then the most recently changed.
            let rows: Vec<PlaylistRow> = sqlx::query_as(concat!(
                "SELECT ",
                playlist_columns!(),
                " FROM playlists p WHERE p.owner_id = $1
                  ORDER BY (p.kind = 1), p.kind, p.updated_at DESC, p.id DESC"
            ))
            .bind(user_id)
            .fetch_all(&core.db)
            .await?;
            ok(ListMyPlaylistsResponse {
                playlists: playlists::to_protos(core, &viewer, &rows, contains).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn list_channel_playlists(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListChannelPlaylistsRequest>,
    ) -> ServiceResult<ListChannelPlaylistsResponse> {
        handle(async {
            let core = &self.core;
            let viewer = Viewer::load(core, ctx.maybe_user_id()?).await?;
            let channel = channels::find_visible(core, &viewer, request.channel_id).await?;
            let page = Page::from_view(request.page.as_option(), 100);
            if channels::shut_for(core, &viewer, channel.owner_id).await? {
                return ok(ListChannelPlaylistsResponse::default());
            }
            let before: Option<(Time, Uuid)> = page.decode()?;
            let mut rows: Vec<PlaylistRow> = sqlx::query_as(concat!(
                "SELECT ",
                playlist_columns!(),
                " FROM playlists p
                  WHERE p.channel_id = $1 AND p.kind = 1 AND p.visibility = 1
                    AND ($2::timestamptz IS NULL OR (p.updated_at, p.id) < ($2, $3))
                  ORDER BY p.updated_at DESC, p.id DESC
                  LIMIT $4"
            ))
            .bind(channel.id)
            .bind(before.map(|(at, _)| at))
            .bind(before.map(|(_, id)| id))
            .bind(page.fetch())
            .fetch_all(&core.db)
            .await?;
            let info = page.finish(&mut rows, |row| (row.updated_at, row.id));
            ok(ListChannelPlaylistsResponse {
                playlists: playlists::to_protos(core, &viewer, &rows, None).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_playlist_items(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListPlaylistItemsRequest>,
    ) -> ServiceResult<ListPlaylistItemsResponse> {
        handle(async {
            let core = &self.core;
            let viewer = Viewer::load(core, ctx.maybe_user_id()?).await?;
            let playlist = playlists::resolve(core, &viewer, request.playlist_id).await?;
            let page = Page::from_view(request.page.as_option(), 100);
            let after: Option<(i64, Uuid)> = page.decode()?;
            let mut rows: Vec<ItemRow> = viewer
                .query_as(concat!(
                    "SELECT ",
                    video_columns!(),
                    ", i.position, i.created_at AS added_at
                     FROM playlist_items i JOIN videos v ON v.id = i.video_id
                     WHERE i.playlist_id = $7 AND ",
                    watchable!(),
                    "  AND ($9::uuid IS NULL OR (i.position, i.video_id) > ($8, $9))
                     ORDER BY i.position, i.video_id
                     LIMIT $10"
                ))
                .bind(playlist.id)
                .bind(after.map(|(position, _)| position))
                .bind(after.map(|(_, id)| id))
                .bind(page.fetch())
                .fetch_all(&core.db)
                .await?;
            let info = page.finish(&mut rows, |row| (row.position, row.video.id));
            viewer.drop_newly_locked(core, &mut rows, |row| row.video.lock_facts()).await?;
            let videos: Vec<VideoRow> = rows.iter().map(|row| row.video.clone()).collect();
            let summaries = videos::summaries(core, &viewer, &videos).await?;
            let items = rows
                .iter()
                .zip(summaries)
                .map(|(row, video)| PlaylistItem {
                    video: video.into(),
                    added_at: ts(row.added_at),
                    ..Default::default()
                })
                .collect();
            ok(ListPlaylistItemsResponse { items, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn add_playlist_item(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AddPlaylistItemRequest>,
    ) -> ServiceResult<AddPlaylistItemResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(core, Some(user_id)).await?;
            // Every change writes rows and the playlist's search document.
            core.cache.rate_limit(&format!("video-playlist-items:{user_id}"), 120, Duration::from_secs(60)).await?;
            let playlist = playlists::resolve_owned(core, user_id, request.playlist_id).await?;
            if playlist.kind == KIND_LIKED {
                return Err(read_only());
            }
            let video = videos::find_watchable(core, &viewer, request.video_id).await?;
            let mut tx = core.db.begin().await?;
            let locked = playlists::find_in(&mut tx, playlist.id)
                .await?
                .or_not_found("PLAYLIST_NOT_FOUND", "playlist not found")?;
            let max = self.cfg.max_playlist_items;
            let present: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM playlist_items WHERE playlist_id = $1 AND video_id = $2)",
            )
            .bind(playlist.id)
            .bind(video.id)
            .fetch_one(&mut *tx)
            .await?;
            ensure!(
                present || i64::from(locked.item_count) < max,
                precondition,
                "PLAYLIST_FULL",
                "a playlist holds at most {max} videos"
            );
            if playlists::add_item_in(&mut tx, playlist.id, video.id).await? {
                playlists::sync_search_in(&mut tx, locked.is_public(), playlist.id).await?;
            }
            tx.commit().await?;
            ok(AddPlaylistItemResponse {
                playlist: self.playlist_response(&viewer, playlist.id).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn remove_playlist_item(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemovePlaylistItemRequest>,
    ) -> ServiceResult<RemovePlaylistItemResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            // Every change writes rows and the playlist's search document.
            core.cache.rate_limit(&format!("video-playlist-items:{user_id}"), 120, Duration::from_secs(60)).await?;
            let playlist = playlists::resolve_owned(core, user_id, request.playlist_id).await?;
            if playlist.kind == KIND_LIKED {
                return Err(read_only());
            }
            // Whatever the video's state now: it can always be taken off the list.
            if let Some(video) = videos::find_ref(core, request.video_id).await? {
                let mut tx = core.db.begin().await?;
                if playlists::remove_item_in(&mut tx, playlist.id, video.id).await? {
                    playlists::sync_search_in(&mut tx, playlist.is_public(), playlist.id).await?;
                }
                tx.commit().await?;
            }
            let viewer = Viewer { user_id: Some(user_id), ..Default::default() };
            ok(RemovePlaylistItemResponse {
                playlist: self.playlist_response(&viewer, playlist.id).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn move_playlist_item(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, MovePlaylistItemRequest>,
    ) -> ServiceResult<MovePlaylistItemResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let playlist = playlists::resolve_owned(core, user_id, request.playlist_id).await?;
            // Built-in lists are ordered by when things were added.
            ensure!(
                playlist.kind == KIND_CUSTOM,
                precondition,
                "PLAYLIST_READ_ONLY",
                "built-in playlists cannot be reordered"
            );
            let missing = || AppError::not_found("PLAYLIST_ITEM_NOT_FOUND", "the video is not in this playlist");
            let video = videos::find_ref(core, request.video_id).await?.ok_or_else(missing)?;
            let after = match request.after_video_id.trim() {
                "" => None,
                reference => Some(videos::find_ref(core, reference).await?.ok_or_else(missing)?.id),
            };
            let mut tx = core.db.begin().await?;
            playlists::find_in(&mut tx, playlist.id).await?.or_not_found("PLAYLIST_NOT_FOUND", "playlist not found")?;
            let moved = playlists::move_item_in(&mut tx, playlist.id, video.id, after).await?;
            ensure!(moved, not_found, "PLAYLIST_ITEM_NOT_FOUND", "the video is not in this playlist");
            tx.commit().await?;
            ok(MovePlaylistItemResponse::default())
        })
        .await
    }
}
