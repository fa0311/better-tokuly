//! `LocationService`: uploads, the friend map, ghost settings, gatherings
//! and live shares. Thin handlers over the pipeline, fan-out, ghost and share
//! modules.
use std::time::Duration;

use buffa::Message as _;
use chrono::Utc;
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult, ServiceStream};
use futures::StreamExt as _;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokuly_core::{prelude::*, realtime};
use tokuly_proto::{
    geo::{geo_event::Event, watch_friend_locations_response::Update, *},
    realtime::ServerFrame,
};

use crate::Geo;

/// Most samples one `ReportLocations` call may carry.
const MAX_SAMPLES: usize = 500;
/// A `WatchFriendLocations` stream ends after this long; clients reopen it.
const MAX_WATCH: Duration = Duration::from_secs(30 * 60);

fn wants_japanese(ctx: &RequestContext) -> bool {
    // Japanese first: only an explicit other language switches away from it.
    let language = ctx.client().language;
    language.is_empty() || language.starts_with("ja")
}

impl LocationService for Geo {
    async fn report_locations(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ReportLocationsRequest>,
    ) -> ServiceResult<ReportLocationsResponse> {
        handle(async {
            let auth = ctx.auth()?;
            let user_id = auth.user_id;
            let req = request.to_owned_message();
            ensure!(
                req.samples.len() <= MAX_SAMPLES,
                invalid,
                "TOO_MANY_SAMPLES",
                "at most {MAX_SAMPLES} samples per upload"
            );
            // One upload a second is plenty; what an upload may contain is
            // bounded by the pipeline (samples at least a second apart).
            self.core.cache.rate_limit(&format!("geo-report:{user_id}"), 60, Duration::from_secs(60)).await?;
            let accepted = self.ingest_batch(user_id, Some(auth.session_id), Utc::now(), &req.samples).await?;
            // The upload is durable once this returns; a ClickHouse outage must not fail it (rows stay buffered).
            if let Err(e) = self.flush_history().await {
                tracing::warn!(error = %e, "location history not written; will retry");
            }
            ok(ReportLocationsResponse {
                moving_interval_seconds: self.cfg.moving_interval_seconds,
                stationary_interval_seconds: self.cfg.stationary_interval_seconds,
                accepted_count: accepted as i32,
                ..Default::default()
            })
        })
        .await
    }

    async fn stop_sharing(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, StopSharingRequest>,
    ) -> ServiceResult<StopSharingResponse> {
        handle(async {
            let req = request.to_owned_message();
            let stop = crate::pipeline::stop_time(req.stopped_at.as_option(), Utc::now());
            let auth = ctx.auth()?;
            Geo::stop_sharing(self, auth.user_id, auth.session_id, stop, req.sequence).await?;
            ok(StopSharingResponse::default())
        })
        .await
    }

    async fn list_friend_locations(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListFriendLocationsRequest>,
    ) -> ServiceResult<ListFriendLocationsResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            ok(ListFriendLocationsResponse {
                locations: self.friend_locations(viewer).await?,
                server_time: ts(Utc::now()),
                ..Default::default()
            })
        })
        .await
    }

    async fn watch_friend_locations(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, WatchFriendLocationsRequest>,
    ) -> ServiceResult<ServiceStream<WatchFriendLocationsResponse>> {
        handle(async {
            let viewer = ctx.user_id()?;
            self.core.cache.rate_limit(&format!("geo-watch:{viewer}"), 30, Duration::from_secs(60)).await?;
            // The same frames the realtime gateway would deliver: subscribe
            // first, then take the snapshot, so no update falls in between.
            let mut frames = self.core.bus.subscribe(realtime::user_subject(viewer)).await?;
            self.mark_watching(viewer).await?;
            let snapshot = ListFriendLocationsResponse {
                locations: self.friend_locations(viewer).await?,
                server_time: ts(Utc::now()),
                ..Default::default()
            };
            let (sender, receiver) = mpsc::channel(64);
            let shutdown = self.core.shutdown.clone();
            let geo = self.clone();
            tokio::spawn(async move {
                let message =
                    |update: Update| WatchFriendLocationsResponse { update: Some(update), ..Default::default() };
                if sender.send(message(Update::Snapshot(Box::new(snapshot)))).await.is_err() {
                    return;
                }
                let timeout = tokio::time::sleep(MAX_WATCH);
                tokio::pin!(timeout);
                let mut still_watching = tokio::time::interval(crate::fanout::WATCHING_TTL / 3);
                loop {
                    let frame = tokio::select! {
                        _ = sender.closed() => return,
                        _ = shutdown.cancelled() => return,
                        _ = &mut timeout => return,
                        _ = still_watching.tick() => {
                            if let Err(e) = geo.mark_watching(viewer).await {
                                tracing::warn!(error = %e, "geo: watcher not refreshed");
                            }
                            continue;
                        }
                        frame = frames.next() => frame,
                    };
                    let Some(frame) = frame else { return };
                    let Ok(frame) = ServerFrame::decode_from_slice(&frame.payload) else { continue };
                    let Some(event) = frame.event::<GeoEvent>() else { continue };
                    let update = match event.event {
                        Some(Event::Location(location)) => Update::Location(location),
                        Some(Event::LocationRemovedUserId(user_id)) => Update::RemovedUserId(user_id),
                        _ => continue,
                    };
                    if sender.send(message(update)).await.is_err() {
                        return;
                    }
                }
            });
            let stream: ServiceStream<WatchFriendLocationsResponse> =
                Box::pin(ReceiverStream::new(receiver).map(Ok::<_, ConnectError>));
            Ok(Response::new(stream))
        })
        .await
    }

    async fn get_my_location(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetMyLocationRequest>,
    ) -> ServiceResult<GetMyLocationResponse> {
        handle(async {
            let (location, sharing) = self.own_location(ctx.user_id()?).await?;
            ok(GetMyLocationResponse { location: location.into(), sharing, ..Default::default() })
        })
        .await
    }

    async fn get_ghost_settings(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetGhostSettingsRequest>,
    ) -> ServiceResult<GetGhostSettingsResponse> {
        handle(async { ok(self.ghost_settings(ctx.user_id()?).await?) }).await
    }

    async fn set_ghost_mode(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetGhostModeRequest>,
    ) -> ServiceResult<SetGhostModeResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            self.core.cache.rate_limit(&format!("geo-ghost:{user_id}"), 60, Duration::from_secs(60)).await?;
            Geo::set_ghost_mode(self, user_id, &req.user_ids, req.mode, req.duration_seconds).await?;
            ok(SetGhostModeResponse { settings: self.ghost_settings(user_id).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn list_gatherings(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListGatheringsRequest>,
    ) -> ServiceResult<ListGatheringsResponse> {
        handle(async {
            ok(ListGatheringsResponse {
                gatherings: Geo::list_gatherings(self, ctx.user_id()?).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn create_live_share(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateLiveShareRequest>,
    ) -> ServiceResult<CreateLiveShareResponse> {
        handle(async {
            let share = Geo::create_live_share(self, ctx.user_id()?, request.duration_seconds, request.label).await?;
            ok(CreateLiveShareResponse { share: share.into(), ..Default::default() })
        })
        .await
    }

    async fn list_live_shares(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListLiveSharesRequest>,
    ) -> ServiceResult<ListLiveSharesResponse> {
        handle(async {
            ok(ListLiveSharesResponse {
                shares: Geo::list_live_shares(self, ctx.user_id()?).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn revoke_live_share(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RevokeLiveShareRequest>,
    ) -> ServiceResult<RevokeLiveShareResponse> {
        handle(async {
            Geo::revoke_live_share(self, ctx.user_id()?, request.share_id).await?;
            ok(RevokeLiveShareResponse::default())
        })
        .await
    }

    async fn get_live_share(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetLiveShareRequest>,
    ) -> ServiceResult<GetLiveShareResponse> {
        handle(async {
            // Public: the token is the permission. A broken sign-in is still reported.
            ctx.maybe_user_id()?;
            ok(Geo::get_live_share(self, request.token, &ctx.client().ip, wants_japanese(&ctx)).await?)
        })
        .await
    }

    async fn watch_live_share(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, WatchLiveShareRequest>,
    ) -> ServiceResult<ServiceStream<WatchLiveShareResponse>> {
        handle(async {
            ctx.maybe_user_id()?;
            let stream = Geo::watch_live_share(self, request.token, &ctx.client().ip, wants_japanese(&ctx)).await?;
            Ok(Response::new(stream))
        })
        .await
    }
}
