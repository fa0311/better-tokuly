//! `MomentService`: thin handlers over `posting`, `viewing`, `calendar`
//! and `settings`.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::moments::*;

use crate::{
    Moments,
    viewing::{Cursor, MAP_LIMIT},
};

impl Moments {
    /// Every read shares one budget per caller.
    async fn limit_reads(&self, user_id: Uuid) -> AppResult<()> {
        self.core.cache.rate_limit(&format!("moments-read:{user_id}"), 240, Duration::from_secs(60)).await
    }
}

impl MomentService for Moments {
    async fn get_today(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetTodayRequest>,
    ) -> ServiceResult<GetTodayResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_reads(user_id).await?;
            let settings = self.settings_of(user_id).await?.to_proto();
            let Some(day) = self.today_of(&self.core.db, user_id).await?.map(|today| today.day) else {
                return ok(GetTodayResponse { settings: settings.into(), ..Default::default() });
            };
            let mine: Option<(Uuid, bool)> =
                sqlx::query_as("SELECT id, deleted_at IS NULL FROM moments WHERE user_id = $1 AND day_id = $2")
                    .bind(user_id)
                    .bind(day.id)
                    .fetch_optional(&self.core.db)
                    .await?;
            let live = match mine {
                Some((moment_id, true)) => self.find_moment(moment_id).await?,
                _ => None,
            };
            let unlocked = live.is_some();
            let rendered = match &live {
                Some(moment) => self.render(user_id, true, std::slice::from_ref(moment)).await?.pop(),
                None => None,
            };
            ok(GetTodayResponse {
                day: self.day_view(&day).into(),
                mine: rendered.into(),
                posted: mine.is_some(),
                unlocked,
                settings: settings.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn post_moment(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, PostMomentRequest>,
    ) -> ServiceResult<PostMomentResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let moment = self.post(user_id, request.to_owned_message()).await?;
            let shown = self.render(user_id, true, std::slice::from_ref(&moment)).await?.pop();
            ok(PostMomentResponse { moment: shown.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_moment(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteMomentRequest>,
    ) -> ServiceResult<DeleteMomentResponse> {
        handle(async {
            self.delete(ctx.user_id()?, parse_id(request.moment_id)?).await?;
            ok(DeleteMomentResponse::default())
        })
        .await
    }

    async fn get_moment(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetMomentRequest>,
    ) -> ServiceResult<GetMomentResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_reads(user_id).await?;
            let moment = self.require_visible(user_id, parse_id(request.moment_id)?).await?;
            let unlocked = moment.user_id == user_id || self.unlocked(user_id).await?;
            let shown = self.render(user_id, unlocked, std::slice::from_ref(&moment)).await?.pop();
            ok(GetMomentResponse { moment: shown.into(), ..Default::default() })
        })
        .await
    }

    async fn list_friend_moments(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListFriendMomentsRequest>,
    ) -> ServiceResult<ListFriendMomentsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_reads(user_id).await?;
            let page = Page::from_view(request.page.as_option(), 100);
            let after: Option<Cursor> = page.decode()?;
            let mut rows = self.friend_moments(user_id, after, page.fetch(), false).await?;
            let info = page.finish(&mut rows, |m| Cursor { posted_at: m.posted_at, id: m.id });
            let unlocked = self.unlocked(user_id).await?;
            ok(ListFriendMomentsResponse {
                moments: self.render(user_id, unlocked, &rows).await?,
                page: info.into(),
                unlocked,
                ..Default::default()
            })
        })
        .await
    }

    async fn list_map_moments(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListMapMomentsRequest>,
    ) -> ServiceResult<ListMapMomentsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_reads(user_id).await?;
            if !self.unlocked(user_id).await? {
                return ok(ListMapMomentsResponse::default());
            }
            let rows = self.friend_moments(user_id, None, MAP_LIMIT, true).await?;
            let moments =
                self.render(user_id, true, &rows).await?.into_iter().filter(|m| m.location.is_set()).collect();
            ok(ListMapMomentsResponse { moments, unlocked: true, ..Default::default() })
        })
        .await
    }

    async fn get_calendar(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetCalendarRequest>,
    ) -> ServiceResult<GetCalendarResponse> {
        handle(async { ok(self.calendar(ctx.user_id()?, request.year, request.month).await?) }).await
    }

    async fn get_moment_settings(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetMomentSettingsRequest>,
    ) -> ServiceResult<GetMomentSettingsResponse> {
        handle(async {
            let settings = self.settings_of(ctx.user_id()?).await?.to_proto();
            ok(GetMomentSettingsResponse { settings: settings.into(), ..Default::default() })
        })
        .await
    }

    async fn update_moment_settings(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateMomentSettingsRequest>,
    ) -> ServiceResult<UpdateMomentSettingsResponse> {
        handle(async {
            let settings = self.update_settings(ctx.user_id()?, request.to_owned_message()).await?.to_proto();
            ok(UpdateMomentSettingsResponse { settings: settings.into(), ..Default::default() })
        })
        .await
    }
}
