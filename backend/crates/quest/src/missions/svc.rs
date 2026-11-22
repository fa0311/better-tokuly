//! `MissionService`: the caller's daily and weekly missions.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::quest::*;

use crate::{Quest, clock};

impl MissionService for Quest {
    async fn list_missions(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListMissionsRequest>,
    ) -> ServiceResult<ListMissionsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let zone = self.caller_zone(user_id).await?;
            let now = now();
            // What the player did a second ago should already show.
            self.flush_user(user_id).await?;
            // Coins of a claim that never reached the wallet are paid now.
            self.pay_unpaid_of(user_id).await;
            let board = self.mission_board(user_id, zone, now).await?;
            let today = clock::local_date(now, zone);
            ok(ListMissionsResponse {
                daily: board.daily.iter().map(|mission| mission.to_proto(zone, now)).collect(),
                weekly: board.weekly.as_ref().map(|mission| mission.to_proto(zone, now)).into(),
                daily_bonus: board.daily_bonus().into(),
                rerolls_left: board.rerolls_left(),
                unclaimed: board.unclaimed.iter().map(|mission| mission.to_proto(zone, now)).collect(),
                daily_resets_at: ts(clock::end_of_day(today, zone)),
                weekly_resets_at: ts(clock::end_of_day(clock::week_end(today), zone)),
                ..Default::default()
            })
        })
        .await
    }

    async fn claim_mission(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ClaimMissionRequest>,
    ) -> ServiceResult<ClaimMissionResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let mission_id = parse_id(request.mission_id)?;
            self.core.cache.rate_limit(&format!("quest:claim:{user_id}"), 60, Duration::from_secs(60)).await?;
            let zone = self.caller_zone(user_id).await?;
            let now = now();
            self.flush_user(user_id).await?;
            let claim = self.collect_mission(user_id, mission_id, zone, now).await?;
            let bonus = claim.bonus.map(|bonus| Reward { coins: bonus.coins, xp: bonus.xp, ..Default::default() });
            ok(ClaimMissionResponse {
                reward: claim.mission.reward().into(),
                mission: claim.mission.to_proto(zone, now).into(),
                bonus: bonus.into(),
                granted: claim.granted,
                progress: self.progress(user_id, zone, now).await?.into(),
                daily_bonus: claim.daily_bonus.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn reroll_mission(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RerollMissionRequest>,
    ) -> ServiceResult<RerollMissionResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let mission_id = parse_id(request.mission_id)?;
            self.core.cache.rate_limit(&format!("quest:reroll:{user_id}"), 20, Duration::from_secs(60)).await?;
            let zone = self.caller_zone(user_id).await?;
            let now = now();
            // The replacement starts from zero, so everything done so far must be counted first.
            self.flush_user(user_id).await?;
            let (mission, rerolls_left) = self.swap_mission(user_id, mission_id, zone, now).await?;
            ok(RerollMissionResponse {
                mission: mission.to_proto(zone, now).into(),
                rerolls_left,
                ..Default::default()
            })
        })
        .await
    }
}
