//! `ShortService`: creating, editing and reading shorts, pins, the saved
//! and liked lists, shares and remix lists.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::{ids::Ref, prelude::*};
use tokuly_proto::{
    common::Visibility,
    shorts::{ShareTarget, *},
};
use tokuly_video::channels;

use crate::{
    Shorts,
    access::{self, FRIENDS, Viewer},
    hydrate,
    shorts::{self, CoverChoice, NewShort, ShortEdit, ShortRow, short_columns},
    sounds::SoundChoice,
};

/// Most shorts one `BatchGetShorts` call may ask for.
const MAX_BATCH: usize = 50;
/// Largest page of the lists in this service.
const MAX_PAGE: i64 = 50;

/// A visibility named in an update: it must be one of the five values.
fn explicit_visibility(value: buffa::EnumValue<Visibility>) -> AppResult<i32> {
    match value.as_known() {
        Some(Visibility::VISIBILITY_UNSPECIFIED) | None => {
            Err(AppError::invalid("VISIBILITY_INVALID", "unknown visibility"))
        }
        Some(known) => Ok(known as i32),
    }
}

/// The cover a client asked for; `None` when the message names none.
fn cover_choice(input: Option<CoverInput>) -> AppResult<Option<CoverChoice>> {
    use cover_input::Cover;
    Ok(match input.and_then(|input| input.cover) {
        None => None,
        Some(Cover::OffsetMs(offset_ms)) => Some(CoverChoice::Frame(offset_ms)),
        Some(Cover::AssetId(asset_id)) => Some(CoverChoice::Image(parse_id(&asset_id)?)),
        Some(Cover::Poster(_)) => Some(CoverChoice::Poster),
    })
}

/// The sound a client asked for; without one the short gets its own
/// original sound, unnamed.
fn sound_choice(input: Option<SoundInput>) -> AppResult<SoundChoice> {
    use sound_input::Sound;
    Ok(match input.and_then(|input| input.sound) {
        None => SoundChoice::Own { title: String::new() },
        Some(Sound::OriginalTitle(title)) => SoundChoice::Own { title },
        Some(Sound::SoundId(sound_id)) => SoundChoice::Existing(
            Uuid::parse_str(sound_id.trim()).map_err(|_| AppError::not_found("SOUND_NOT_FOUND", "sound not found"))?,
        ),
        Some(Sound::External(track)) => {
            SoundChoice::External { title: track.title.clone(), artist: track.artist.clone() }
        }
    })
}

impl Shorts {
    /// The caller with their friends and blocks, or an anonymous viewer.
    pub(crate) async fn viewer(&self, ctx: &RequestContext) -> AppResult<Viewer> {
        Viewer::load(&self.core, ctx.maybe_user_id()?).await
    }

    /// A short of `user_id` as its owner sees it.
    async fn own_short(&self, user_id: Uuid, row: &ShortRow) -> AppResult<Short> {
        let owner = Viewer { user_id: Some(user_id), ..Default::default() };
        hydrate::short(&self.core, &owner, row).await
    }

    /// The short a client named for an action that must behave the same
    /// whether the short exists, is gone or is hidden from them (taking it
    /// off one of their lists): its UUID, or `None` when a short ID names
    /// nothing. Only a reference that is no ID at all is an error
    /// (`INVALID_ID`): an answer that told "no such short" from "not yours
    /// to see" would let anyone ask whether a short still exists.
    pub(crate) async fn named_short(&self, reference: &str) -> AppResult<Option<Uuid>> {
        match Ref::parse(reference.trim())? {
            Ref::Id(id) => Ok(Some(id)),
            short_id => Ok(shorts::find(&self.core, &short_id).await?.map(|short| short.id)),
        }
    }
}

impl ShortService for Shorts {
    async fn create_short(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateShortRequest>,
    ) -> ServiceResult<CreateShortResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let channel = match req.channel_id.trim() {
                "" => channels::default_for(core, user_id).await?.ok_or_else(|| {
                    AppError::precondition("CHANNEL_REQUIRED", "create a channel before posting shorts")
                })?,
                reference => channels::require_owned_ref(core, user_id, reference).await?,
            };
            let source = match req.source.into_option() {
                Some(source) => {
                    let subject_type = source.r#type.as_known().ok_or_else(|| {
                        AppError::invalid("SOURCE_INVALID", "a short can be made from a clip, a video or a live stream")
                    })?;
                    Some((subject_type, source.id))
                }
                None => None,
            };
            let new = NewShort {
                channel,
                asset_id: parse_id(&req.asset_id)?,
                caption: req.caption,
                visibility: shorts::validate_visibility(req.visibility, FRIENDS)?,
                comments_enabled: req.comments_enabled.unwrap_or(true),
                allow_remix: req.allow_remix.unwrap_or(true),
                allow_duet: req.allow_duet.unwrap_or(true),
                notify_subscribers: req.notify_subscribers.unwrap_or(true),
                cover: cover_choice(req.cover.into_option())?.unwrap_or(CoverChoice::Poster),
                sound: sound_choice(req.sound.into_option())?,
                source,
                remix: req.remix.into_option().map(|remix| (remix.short_id, remix.kind.to_i32())),
            };
            core.cache.rate_limit(&format!("shorts-create:{user_id}"), 30, Duration::from_secs(3600)).await?;
            let short_id = self.create(new).await?;
            let row = shorts::find_by_id(core, short_id).await?.ok_or_else(shorts::not_found)?;
            ok(CreateShortResponse { short: self.own_short(user_id, &row).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn update_short(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateShortRequest>,
    ) -> ServiceResult<UpdateShortResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let short = shorts::find_owned(&self.core, user_id, &req.short_id).await?;
            let hourly = Duration::from_secs(3600);
            self.core.cache.rate_limit(&format!("shorts-update:{user_id}"), 120, hourly).await?;
            let cover = cover_choice(req.cover.into_option())?;
            if matches!(cover, Some(CoverChoice::Frame(_))) {
                // Each frame is a download and an ffmpeg run on a worker.
                self.core.cache.rate_limit(&format!("shorts-cover:{user_id}"), 20, hourly).await?;
            }
            let edit = ShortEdit {
                caption: req.caption,
                visibility: req.visibility.map(explicit_visibility).transpose()?,
                comments_enabled: req.comments_enabled,
                allow_remix: req.allow_remix,
                allow_duet: req.allow_duet,
                cover,
            };
            let row = self.edit(&short, edit).await?;
            ok(UpdateShortResponse { short: self.own_short(user_id, &row).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_short(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteShortRequest>,
    ) -> ServiceResult<DeleteShortResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let short = shorts::find_owned(&self.core, user_id, request.short_id).await?;
            shorts::delete_by_owner(&self.core, &short).await?;
            ok(DeleteShortResponse::default())
        })
        .await
    }

    async fn get_short(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetShortRequest>,
    ) -> ServiceResult<GetShortResponse> {
        handle(async {
            let core = &self.core;
            let viewer = self.viewer(&ctx).await?;
            let row = shorts::find_ref(core, request.short_id).await?.ok_or_else(shorts::not_found)?;
            let facts = row.facts();
            // Moderators review what was reported, also after its owner
            // deleted it; never their own shorts.
            let reviewing = !access::can_watch(&viewer, &facts)
                && ctx
                    .maybe_auth()?
                    .is_some_and(|auth| auth.has_role("moderator") && access::moderator_sees(auth.user_id, &facts));
            ensure!(access::can_watch(&viewer, &facts) || reviewing, not_found, "SHORT_NOT_FOUND", "short not found");
            let mut short = hydrate::short(core, &viewer, &row).await?;
            if reviewing {
                // What they review is the video, whatever state it is in.
                short.playback = row.playback().into();
            }
            ok(GetShortResponse { short: short.into(), ..Default::default() })
        })
        .await
    }

    async fn batch_get_shorts(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, BatchGetShortsRequest>,
    ) -> ServiceResult<BatchGetShortsResponse> {
        handle(async {
            let core = &self.core;
            let viewer = self.viewer(&ctx).await?;
            let req = request.to_owned_message();
            ensure!(
                req.short_ids.len() <= MAX_BATCH,
                invalid,
                "TOO_MANY_SHORTS",
                "at most {MAX_BATCH} shorts per call"
            );
            let mut rows = Vec::with_capacity(req.short_ids.len());
            for reference in &req.short_ids {
                if let Some(row) = shorts::find_ref(core, reference).await?
                    && access::can_watch(&viewer, &row.facts())
                {
                    rows.push(row);
                }
            }
            ok(BatchGetShortsResponse { shorts: hydrate::shorts(core, &viewer, &rows).await?, ..Default::default() })
        })
        .await
    }

    async fn list_my_shorts(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListMyShortsRequest>,
    ) -> ServiceResult<ListMyShortsResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let channel_id = match request.channel_id.trim() {
                "" => None,
                reference => Some(channels::require_owned_ref(core, user_id, reference).await?.id),
            };
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let before: Option<Uuid> = page.decode()?;
            let mut rows: Vec<ShortRow> = sqlx::query_as(concat!(
                "SELECT ",
                short_columns!(),
                " FROM shorts s
                  WHERE s.owner_id = $1 AND s.status <> 4 AND ($2::uuid IS NULL OR s.channel_id = $2)
                    AND ($3 = 0 OR s.status = $3)
                    AND ($4::uuid IS NULL OR s.id < $4)
                  ORDER BY s.id DESC LIMIT $5"
            ))
            .bind(user_id)
            .bind(channel_id)
            .bind(request.status.to_i32())
            .bind(before)
            .bind(page.fetch())
            .fetch_all(&core.db)
            .await?;
            let info = page.finish(&mut rows, |row| row.id);
            let owner = Viewer { user_id: Some(user_id), ..Default::default() };
            ok(ListMyShortsResponse {
                shorts: hydrate::shorts(core, &owner, &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn pin_short(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, PinShortRequest>,
    ) -> ServiceResult<PinShortResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let short = shorts::find_owned(&self.core, user_id, request.short_id).await?;
            let row = shorts::pin(&self.core, &short).await?;
            ok(PinShortResponse { short: self.own_short(user_id, &row).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn unpin_short(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnpinShortRequest>,
    ) -> ServiceResult<UnpinShortResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let short = shorts::find_owned(&self.core, user_id, request.short_id).await?;
            let row = shorts::unpin(&self.core, &short).await?;
            ok(UnpinShortResponse { short: self.own_short(user_id, &row).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn save_short(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SaveShortRequest>,
    ) -> ServiceResult<SaveShortResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(&self.core, Some(user_id)).await?;
            let short = shorts::find_watchable(&self.core, &viewer, request.short_id).await?;
            // Drafts are not content yet, not even for their owner's lists.
            ensure!(short.is_published(), not_found, "SHORT_NOT_FOUND", "short not found");
            self.core.cache.rate_limit(&format!("shorts-save:{user_id}"), 120, Duration::from_secs(60)).await?;
            ok(SaveShortResponse { save_count: self.save(user_id, &short).await?, ..Default::default() })
        })
        .await
    }

    async fn unsave_short(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnsaveShortRequest>,
    ) -> ServiceResult<UnsaveShortResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(&self.core, Some(user_id)).await?;
            let short = match self.named_short(request.short_id).await? {
                Some(short_id) => self.unsave(user_id, short_id).await?,
                None => None,
            };
            // The count of a short the caller cannot see is not theirs to
            // learn: such a short answers like one that is gone.
            let save_count = short
                .filter(|short| access::can_watch(&viewer, &short.facts()))
                .map(|short| short.save_count)
                .unwrap_or(0);
            ok(UnsaveShortResponse { save_count, ..Default::default() })
        })
        .await
    }

    async fn list_saved_shorts(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListSavedShortsRequest>,
    ) -> ServiceResult<ListSavedShortsResponse> {
        handle(async {
            let viewer = Viewer::load(&self.core, Some(ctx.user_id()?)).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (rows, info) = self.saved_page(&viewer, &page).await?;
            ok(ListSavedShortsResponse {
                shorts: hydrate::shorts(&self.core, &viewer, &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_liked_shorts(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListLikedShortsRequest>,
    ) -> ServiceResult<ListLikedShortsResponse> {
        handle(async {
            let viewer = Viewer::load(&self.core, Some(ctx.user_id()?)).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let (rows, info) = self.liked_page(&viewer, &page).await?;
            ok(ListLikedShortsResponse {
                shorts: hydrate::shorts(&self.core, &viewer, &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn share_short(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ShareShortRequest>,
    ) -> ServiceResult<ShareShortResponse> {
        handle(async {
            let core = &self.core;
            let viewer = self.viewer(&ctx).await?;
            let target = match request.target.as_known() {
                Some(ShareTarget::SHARE_TARGET_UNSPECIFIED) => ShareTarget::SHARE_TARGET_OTHER,
                Some(known) => known,
                None => bail!(invalid, "SHARE_TARGET_INVALID", "unknown share target"),
            };
            let short = shorts::find_watchable(core, &viewer, request.short_id).await?;
            ensure!(short.is_published(), not_found, "SHORT_NOT_FOUND", "short not found");
            let ip = ctx.client().ip.to_string();
            match viewer.user_id {
                Some(user_id) => {
                    core.cache.rate_limit(&format!("shorts-share:{user_id}"), 60, Duration::from_secs(60)).await?
                }
                None => core.cache.rate_limit(&format!("shorts-share:ip:{ip}"), 30, Duration::from_secs(60)).await?,
            }
            let sharer = crate::signals::viewer_key(core, viewer.user_id, &ip);
            let share_count = self.share(viewer.user_id, &sharer, &short, target as i32).await?;
            ok(ShareShortResponse { url: core.web_url(&short.path()), share_count, ..Default::default() })
        })
        .await
    }

    async fn list_remixes(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListRemixesRequest>,
    ) -> ServiceResult<ListRemixesResponse> {
        handle(async {
            let core = &self.core;
            let viewer = self.viewer(&ctx).await?;
            let original = shorts::find_watchable(core, &viewer, request.short_id).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let before: Option<(Time, Uuid)> = page.decode()?;
            let mut rows: Vec<ShortRow> = sqlx::query_as(concat!(
                "SELECT ",
                short_columns!(),
                " FROM shorts s WHERE s.remix_of_id = $1 AND ",
                access::public!(2),
                " AND ($3::timestamptz IS NULL OR (s.published_at, s.id) < ($3, $4))
                  ORDER BY s.published_at DESC, s.id DESC LIMIT $5"
            ))
            .bind(original.id)
            .bind(&viewer.blocked)
            .bind(before.map(|(at, _)| at))
            .bind(before.map(|(_, id)| id))
            .bind(page.fetch())
            .fetch_all(&core.db)
            .await?;
            let info = page.finish(&mut rows, |row| (row.published_at.unwrap_or(row.created_at), row.id));
            ok(ListRemixesResponse {
                shorts: hydrate::shorts(core, &viewer, &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }
}
