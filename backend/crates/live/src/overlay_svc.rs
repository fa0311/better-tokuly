//! `OverlayService`: OBS widgets.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::live::{overlay_settings::Widget, *};
use tokuly_video::channels;

use crate::{
    Live, chat,
    overlays::{self, OverlayEdit},
    polls, streams,
};

impl OverlayService for Live {
    async fn list_overlay_presets(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, ListOverlayPresetsRequest>,
    ) -> ServiceResult<ListOverlayPresetsResponse> {
        handle(async { ok(ListOverlayPresetsResponse { presets: overlays::presets_proto(), ..Default::default() }) })
            .await
    }

    async fn list_overlays(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListOverlaysRequest>,
    ) -> ServiceResult<ListOverlaysResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let channel = channels::require_owned_ref(core, user_id, request.channel_id).await?;
            let rows = overlays::of_channel(core, channel.id).await?;
            let overlays = rows.iter().map(|row| row.to_proto(core, true)).collect::<AppResult<Vec<_>>>()?;
            ok(ListOverlaysResponse { overlays, ..Default::default() })
        })
        .await
    }

    async fn create_overlay(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateOverlayRequest>,
    ) -> ServiceResult<CreateOverlayResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let channel = channels::require_owned_ref(core, user_id, &req.channel_id).await?;
            let kind = overlays::validate_kind(req.kind)?;
            let edit = OverlayEdit {
                name: Some(req.name),
                preset: Some(req.preset),
                custom_css: Some(req.custom_css),
                settings: req.settings.into_option(),
                reset_goal: false,
            };
            let row = self.add_overlay(&channel, kind, edit).await?;
            ok(CreateOverlayResponse { overlay: Some(row.to_proto(core, true)?).into(), ..Default::default() })
        })
        .await
    }

    async fn update_overlay(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateOverlayRequest>,
    ) -> ServiceResult<UpdateOverlayResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let row = overlays::find_owned(core, user_id, &req.overlay_id).await?;
            // Every change makes the open widget reload.
            core.cache.rate_limit(&format!("live-overlay-edit:{user_id}"), 120, Duration::from_secs(60)).await?;
            let edit = OverlayEdit {
                name: req.name,
                preset: req.preset,
                custom_css: req.custom_css,
                settings: req.settings.into_option(),
                reset_goal: req.reset_goal,
            };
            let updated = self.change_overlay(&row, edit).await?;
            ok(UpdateOverlayResponse { overlay: Some(updated.to_proto(core, true)?).into(), ..Default::default() })
        })
        .await
    }

    async fn delete_overlay(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteOverlayRequest>,
    ) -> ServiceResult<DeleteOverlayResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let row = overlays::find_owned(&self.core, user_id, request.overlay_id).await?;
            self.remove_overlay(&row).await?;
            ok(DeleteOverlayResponse::default())
        })
        .await
    }

    async fn rotate_overlay_token(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RotateOverlayTokenRequest>,
    ) -> ServiceResult<RotateOverlayTokenResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let row = overlays::find_owned(core, user_id, request.overlay_id).await?;
            core.cache.rate_limit(&format!("live-overlay-edit:{user_id}"), 120, Duration::from_secs(60)).await?;
            let rotated = self.renew_overlay_token(&row).await?;
            ok(RotateOverlayTokenResponse { overlay: Some(rotated.to_proto(core, true)?).into(), ..Default::default() })
        })
        .await
    }

    async fn get_overlay(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetOverlayRequest>,
    ) -> ServiceResult<GetOverlayResponse> {
        handle(async {
            let core = &self.core;
            core.cache.rate_limit(&format!("live-overlay:{}", ctx.client().ip), 120, Duration::from_secs(60)).await?;
            let token = request.token.trim();
            let row = overlays::find_by_token(core, token)
                .await?
                .ok_or_else(|| AppError::not_found("OVERLAY_NOT_FOUND", "overlay not found"))?;
            let channel = channels::summaries(core, &[row.channel_id]).await?.remove(&row.channel_id);
            let stream = streams::current_of_channel(core, row.channel_id).await?;
            let settings = row.settings();
            let mut response = GetOverlayResponse {
                overlay: Some(row.to_proto(core, false)?).into(),
                css: row.css(),
                channel: channel.into(),
                realtime_topic: row.topic(),
                goal: overlays::goal_progress(core, &row).await?.into(),
                ..Default::default()
            };
            if let Some(stream) = &stream {
                response.stream = Some(streams::summary(core, stream).await?).into();
                match &settings.widget {
                    Some(Widget::ChatBox(chat_box)) => {
                        let mut recent =
                            chat::list(core, stream.id, None, &[], i64::from(chat_box.max_messages.clamp(1, 50)))
                                .await?;
                        recent.reverse();
                        response.recent_messages = chat::to_protos(core, stream, &recent).await?;
                    }
                    Some(Widget::Poll(poll_widget)) => {
                        let poll = match polls::open_of(core, stream.id).await? {
                            Some(open) => Some(open),
                            None if poll_widget.hide_when_idle => None,
                            None => polls::latest_of(core, stream.id).await?,
                        };
                        response.poll = poll.map(|poll| poll.to_proto(0)).into();
                    }
                    _ => {}
                }
            }
            ok(response)
        })
        .await
    }

    async fn send_test_alert(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SendTestAlertRequest>,
    ) -> ServiceResult<SendTestAlertResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let channel = channels::require_owned_ref(core, user_id, request.channel_id).await?;
            let kind = request
                .kind
                .as_known()
                .ok_or_else(|| AppError::invalid("ALERT_KIND_INVALID", "unknown kind of alert"))?;
            core.cache.rate_limit(&format!("live-test-alert:{user_id}"), 30, Duration::from_secs(60)).await?;
            ok(SendTestAlertResponse {
                overlay_count: self.play_test_alert(&channel, kind).await?,
                ..Default::default()
            })
        })
        .await
    }
}
