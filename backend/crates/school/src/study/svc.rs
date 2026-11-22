//! `StudyService`: thin RPC handlers over the focus timer, the study log,
//! the statistics and the presence of friends.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::school::*;

use crate::School;

impl School {
    fn heartbeat_interval(&self) -> i32 {
        self.cfg.heartbeat_interval_seconds as i32
    }

    fn idle_timeout(&self) -> i32 {
        self.cfg.session_idle_seconds as i32
    }

    /// Bounds the reads that add up study time or look at friends' sessions.
    async fn limit_study_reads(&self, user_id: Uuid) -> AppResult<()> {
        self.core.cache.rate_limit(&format!("school:stats:{user_id}"), 240, Duration::from_secs(60)).await
    }
}

impl StudyService for School {
    async fn start_study_session(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, StartStudySessionRequest>,
    ) -> ServiceResult<StartStudySessionResponse> {
        handle(async {
            let session = self.start_session(ctx.user_id()?, &request.to_owned_message()).await?;
            ok(StartStudySessionResponse {
                session: session.into(),
                heartbeat_interval_seconds: self.heartbeat_interval(),
                idle_timeout_seconds: self.idle_timeout(),
                ..Default::default()
            })
        })
        .await
    }

    async fn pause_study_session(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, PauseStudySessionRequest>,
    ) -> ServiceResult<PauseStudySessionResponse> {
        handle(async {
            let session = self.pause_session(ctx.user_id()?).await?;
            ok(PauseStudySessionResponse { session: session.into(), ..Default::default() })
        })
        .await
    }

    async fn resume_study_session(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ResumeStudySessionRequest>,
    ) -> ServiceResult<ResumeStudySessionResponse> {
        handle(async {
            let session = self.resume_session(ctx.user_id()?).await?;
            ok(ResumeStudySessionResponse { session: session.into(), ..Default::default() })
        })
        .await
    }

    async fn stop_study_session(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, StopStudySessionRequest>,
    ) -> ServiceResult<StopStudySessionResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let session = self.stop_session(user_id, request.memo).await?;
            let progress = self.study_progress(user_id, self.now()).await?;
            ok(StopStudySessionResponse { session: session.into(), progress: progress.into(), ..Default::default() })
        })
        .await
    }

    async fn heartbeat_study_session(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, HeartbeatStudySessionRequest>,
    ) -> ServiceResult<HeartbeatStudySessionResponse> {
        handle(async {
            let session = self.heartbeat(ctx.user_id()?).await?;
            ok(HeartbeatStudySessionResponse {
                session: session.into(),
                heartbeat_interval_seconds: self.heartbeat_interval(),
                ..Default::default()
            })
        })
        .await
    }

    async fn skip_study_phase(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, SkipStudyPhaseRequest>,
    ) -> ServiceResult<SkipStudyPhaseResponse> {
        handle(async {
            let session = self.skip_phase(ctx.user_id()?).await?;
            ok(SkipStudyPhaseResponse { session: session.into(), ..Default::default() })
        })
        .await
    }

    async fn get_current_study_session(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetCurrentStudySessionRequest>,
    ) -> ServiceResult<GetCurrentStudySessionResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_reads(user_id).await?;
            let session = self.current_session(user_id).await?;
            ok(GetCurrentStudySessionResponse {
                session: session.into(),
                heartbeat_interval_seconds: self.heartbeat_interval(),
                idle_timeout_seconds: self.idle_timeout(),
                ..Default::default()
            })
        })
        .await
    }

    async fn log_study_session(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, LogStudySessionRequest>,
    ) -> ServiceResult<LogStudySessionResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.core.cache.rate_limit(&format!("school:log:{user_id}"), 60, Duration::from_secs(3600)).await?;
            let session = self.log_session(user_id, &request.to_owned_message()).await?;
            ok(LogStudySessionResponse { session: session.into(), ..Default::default() })
        })
        .await
    }

    async fn update_study_session(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateStudySessionRequest>,
    ) -> ServiceResult<UpdateStudySessionResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            // Re-labelling a session that is going is announced to friends.
            self.core.cache.rate_limit(&format!("school:session-edit:{user_id}"), 60, Duration::from_secs(60)).await?;
            let session = self.update_session(user_id, &request.to_owned_message()).await?;
            ok(UpdateStudySessionResponse { session: session.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_study_session(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteStudySessionRequest>,
    ) -> ServiceResult<DeleteStudySessionResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_writes(user_id).await?;
            self.delete_session(user_id, parse_id(request.session_id)?).await?;
            ok(DeleteStudySessionResponse::default())
        })
        .await
    }

    async fn list_study_sessions(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListStudySessionsRequest>,
    ) -> ServiceResult<ListStudySessionsResponse> {
        handle(async {
            let request = request.to_owned_message();
            let page = Page::from_request(request.page.as_option(), 100);
            let user_id = ctx.user_id()?;
            self.limit_study_reads(user_id).await?;
            let (sessions, page) = self.list_sessions(user_id, &request, &page).await?;
            ok(ListStudySessionsResponse { sessions, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn get_study_summary(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetStudySummaryRequest>,
    ) -> ServiceResult<GetStudySummaryResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_study_reads(user_id).await?;
            let period = request.period.as_known().unwrap_or(StatsPeriod::STATS_PERIOD_UNSPECIFIED);
            ok(self.study_summary(user_id, period, request.anchor_date).await?)
        })
        .await
    }

    async fn get_study_progress(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetStudyProgressRequest>,
    ) -> ServiceResult<GetStudyProgressResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_study_reads(user_id).await?;
            let progress = self.study_progress(user_id, self.now()).await?;
            ok(GetStudyProgressResponse { progress: progress.into(), ..Default::default() })
        })
        .await
    }

    async fn get_study_heatmap(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetStudyHeatmapRequest>,
    ) -> ServiceResult<GetStudyHeatmapResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            self.limit_study_reads(viewer).await?;
            let owner = parse_opt_id(request.user_id)?.unwrap_or(viewer);
            ok(self.study_heatmap(viewer, owner, request.end_date, request.days).await?)
        })
        .await
    }

    async fn list_studying_friends(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListStudyingFriendsRequest>,
    ) -> ServiceResult<ListStudyingFriendsResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            self.limit_study_reads(viewer).await?;
            let friends = self.studying_friends(viewer, self.now()).await?;
            ok(ListStudyingFriendsResponse { friends, ..Default::default() })
        })
        .await
    }

    async fn get_study_ranking(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetStudyRankingRequest>,
    ) -> ServiceResult<GetStudyRankingResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            self.limit_study_reads(viewer).await?;
            ok(self.study_ranking(viewer, request.anchor_date).await?)
        })
        .await
    }

    async fn cheer_study(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CheerStudyRequest>,
    ) -> ServiceResult<CheerStudyResponse> {
        handle(async {
            let cheer = self.cheer(ctx.user_id()?, parse_id(request.user_id)?).await?;
            ok(CheerStudyResponse { cheer: cheer.into(), ..Default::default() })
        })
        .await
    }

    async fn list_study_cheers(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListStudyCheersRequest>,
    ) -> ServiceResult<ListStudyCheersResponse> {
        handle(async {
            let page = Page::from_view(request.page.as_option(), 100);
            let user_id = ctx.user_id()?;
            self.limit_study_reads(user_id).await?;
            let (cheers, page) = self.list_cheers(user_id, parse_id(request.session_id)?, &page).await?;
            ok(ListStudyCheersResponse { cheers, page: page.into(), ..Default::default() })
        })
        .await
    }
}
