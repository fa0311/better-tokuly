//! `PushService`: registering the devices push notifications go to.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::{outbound::Localized, prelude::*};
use tokuly_proto::notify::*;

use crate::{Notify, categories, delivery::PushJob, devices, inbox};

/// Kind of the notification `SendTestPush` sends.
pub const TEST_KIND: &str = "system.test";

impl PushService for Notify {
    async fn get_push_config(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, GetPushConfigRequest>,
    ) -> ServiceResult<GetPushConfigResponse> {
        handle(async {
            let mut channels: Vec<i32> = self.channels.keys().copied().collect();
            channels.sort_unstable();
            ok(GetPushConfigResponse {
                vapid_public_key: self.vapid.public_key().to_owned(),
                channels: channels.into_iter().map(Into::into).collect(),
                ..Default::default()
            })
        })
        .await
    }

    async fn register_device(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RegisterDeviceRequest>,
    ) -> ServiceResult<RegisterDeviceResponse> {
        handle(async {
            let auth = ctx.auth()?;
            let (user_id, session_id) = (auth.user_id, auth.session_id);
            // Apps re-register on every launch; this only stops loops.
            self.core.cache.rate_limit(&format!("push-register:{user_id}"), 60, Duration::from_secs(3600)).await?;
            let mut device = devices::validate(&self.endpoint_policy, request.to_owned_message())?;
            ensure!(
                self.channels.contains_key(&(device.channel as i32)),
                unimplemented,
                "PUSH_CHANNEL_UNAVAILABLE",
                "this server is not set up to deliver through that push channel"
            );
            // The registration lives and dies with the session; an access
            // token can outlast its session by a few minutes.
            let session_device: Option<String> = sqlx::query_scalar("SELECT device_name FROM sessions WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL AND expires_at > now()")
                .bind(session_id)
                .bind(user_id)
                .fetch_optional(&self.core.db)
                .await?;
            let Some(session_device) = session_device else {
                bail!(unauthenticated, "SESSION_REVOKED", "this session has ended");
            };
            if device.device_name.is_empty() {
                device.device_name = session_device;
            }
            let row = devices::register(&self.core, user_id, session_id, &device).await?;
            ok(RegisterDeviceResponse { device: row.to_proto(session_id).into(), ..Default::default() })
        })
        .await
    }

    async fn unregister_device(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnregisterDeviceRequest>,
    ) -> ServiceResult<UnregisterDeviceResponse> {
        handle(async {
            let auth = ctx.auth()?;
            match parse_opt_id(request.device_id)? {
                None => {
                    sqlx::query("DELETE FROM push_devices WHERE session_id = $1 AND user_id = $2")
                        .bind(auth.session_id)
                        .bind(auth.user_id)
                        .execute(&self.core.db)
                        .await?;
                }
                Some(device_id) => {
                    // Scoped to the caller: other people's registrations look
                    // like they do not exist.
                    let done =
                        sqlx::query("DELETE FROM push_devices WHERE id = $1 AND user_id = $2")
                            .bind(device_id)
                            .bind(auth.user_id)
                            .execute(&self.core.db)
                            .await?;
                    ensure!(
                        done.rows_affected() > 0,
                        not_found,
                        "DEVICE_NOT_FOUND",
                        "push registration not found"
                    );
                }
            }
            ok(UnregisterDeviceResponse::default())
        })
        .await
    }

    async fn list_devices(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListDevicesRequest>,
    ) -> ServiceResult<ListDevicesResponse> {
        handle(async {
            let auth = ctx.auth()?;
            let devices = devices::list(&self.core, auth.user_id).await?;
            ok(ListDevicesResponse {
                devices: devices
                    .iter()
                    .map(|d| d.to_proto(auth.session_id))
                    .collect(),
                ..Default::default()
            })
        })
        .await
    }

    async fn send_test_push(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, SendTestPushRequest>,
    ) -> ServiceResult<SendTestPushResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.core
                .cache
                .rate_limit(&format!("push-test:{user_id}"), 5, Duration::from_secs(60))
                .await?;
            let device_count = devices::list(&self.core, user_id).await?.len();
            ensure!(
                device_count > 0,
                precondition,
                "NO_PUSH_DEVICES",
                "no device is registered for push notifications"
            );
            // Straight to the delivery job: a test is about the devices, so
            // preferences and quiet hours do not apply and the inbox stays clean.
            let job = PushJob {
                id: new_id(),
                user_id,
                notification_id: None,
                kind: TEST_KIND.to_owned(),
                category: categories::of_kind(TEST_KIND).id.to_owned(),
                title: Localized::new("テスト通知だよ！", "Test notification"),
                body: Localized::new(
                    "プッシュ通知はちゃんと届いてるよ🎉",
                    "Push notifications are working 🎉",
                ),
                image_url: None,
                path: "/notifications/settings".to_owned(),
                collapse_key: Some("notify:test".to_owned()),
                badge: i64::from(inbox::unread_count(&self.core.db, user_id).await?.total),
                data: serde_json::Value::Null,
                refs: Vec::new(),
                urgent: false,
                created_at: now(),
            };
            self.enqueue_push(&job).await?;
            ok(SendTestPushResponse {
                device_count: device_count.min(i32::MAX as usize) as i32,
                ..Default::default()
            })
        })
        .await
    }
}
