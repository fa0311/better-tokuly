//! `StreakService`: the caller's streaks with their friends.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::quest::*;
use tokuly_social::graph;

use crate::{Quest, clock};

/// Largest page of streaks.
const MAX_PAGE: i64 = 100;

impl StreakService for Quest {
    async fn list_streaks(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListStreaksRequest>,
    ) -> ServiceResult<ListStreaksResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE);
            let today = clock::local_date(now(), self.caller_zone(user_id).await?);
            self.finish_restores(user_id).await;
            let (streaks, page) = self.streaks_page(user_id, &page).await?;
            ok(ListStreaksResponse {
                streaks,
                page: page.into(),
                restore: self.restore_offer(user_id, today).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_streak(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetStreakRequest>,
    ) -> ServiceResult<GetStreakResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let friend = parse_id(request.user_id)?;
            ensure!(
                friend != user_id && graph::are_friends(&self.core, user_id, friend).await?,
                precondition,
                "NOT_FRIENDS",
                "you are not friends with this user"
            );
            let today = clock::local_date(now(), self.caller_zone(user_id).await?);
            self.finish_restores(user_id).await;
            ok(GetStreakResponse {
                streak: self.streak_between(user_id, friend).await?.into(),
                restore: self.restore_offer(user_id, today).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn restore_streak(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RestoreStreakRequest>,
    ) -> ServiceResult<RestoreStreakResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let friend = parse_id(request.user_id)?;
            self.core.cache.rate_limit(&format!("quest:restore:{user_id}"), 10, Duration::from_secs(60)).await?;
            let today = clock::local_date(now(), self.caller_zone(user_id).await?);
            let (streak, coins_spent) = self.bring_back_streak(user_id, friend, today).await?;
            ok(RestoreStreakResponse { streak: streak.into(), coins_spent: coins_spent as i32, ..Default::default() })
        })
        .await
    }
}
