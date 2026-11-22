//! `AuthorizationService`: the user's side of OAuth — the consent screen,
//! the device page and the list of applications with access.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::dev::*;

use crate::{Dev, scopes};

const MAX_PAGE_SIZE: i64 = 100;
/// The window of the consent limits below.
const CONSENT_WINDOW: Duration = Duration::from_secs(300);

impl Dev {
    /// Holds one person to a number of consent requests per five minutes.
    /// Looking at a consent screen is cheap; approving writes a grant,
    /// announces it and mints a code, so it gets the smaller budget.
    async fn limit_consent(&self, user_id: Uuid, what: &str, max: u32) -> AppResult<()> {
        self.core.cache.rate_limit(&format!("dev-consent-{what}:{user_id}"), max, CONSENT_WINDOW).await
    }
}

fn authorization_request(request: Option<&AuthorizationRequest>) -> AppResult<AuthorizationRequest> {
    request
        .cloned()
        .ok_or_else(|| AppError::invalid("AUTHORIZATION_REQUEST_INVALID", "the authorization request is missing"))
}

impl AuthorizationService for Dev {
    async fn get_public_app(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, GetPublicAppRequest>,
    ) -> ServiceResult<GetPublicAppResponse> {
        handle(async {
            let app = match Uuid::parse_str(request.app_id) {
                Ok(app_id) => self.find_public_app(app_id).await?,
                Err(_) => None,
            };
            ok(GetPublicAppResponse {
                app: app.or_not_found("APP_NOT_FOUND", "application not found")?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_consent(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetConsentRequest>,
    ) -> ServiceResult<GetConsentResponse> {
        handle(async {
            let user_id = ctx.require_session()?.user_id;
            self.limit_consent(user_id, "view", 120).await?;
            let checked = self
                .check_authorization_request(&authorization_request(request.to_owned_message().request.as_option())?)
                .await?;
            let result = match checked.outcome() {
                Ok((scopes, _)) => get_consent_response::Result::from(
                    self.consent(&checked.app, scopes, user_id, Some(&checked.redirect_uri)).await?,
                ),
                Err(error) => get_consent_response::Result::RedirectUrl(checked.error_redirect(error)),
            };
            ok(GetConsentResponse { result: Some(result), ..Default::default() })
        })
        .await
    }

    async fn approve_authorization(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ApproveAuthorizationRequest>,
    ) -> ServiceResult<ApproveAuthorizationResponse> {
        handle(async {
            let user_id = ctx.require_session()?.user_id;
            self.limit_consent(user_id, "approve", 30).await?;
            let checked = self
                .check_authorization_request(&authorization_request(request.to_owned_message().request.as_option())?)
                .await?;
            let redirect_url = self.approve_authorization(user_id, &checked).await?;
            let attrs = serde_json::json!({ "app_id": checked.app.id, "name": checked.app.name });
            self.identity.log_security_event(user_id, "APP_AUTHORIZED", &ctx.client(), "", attrs).await;
            ok(ApproveAuthorizationResponse { redirect_url, ..Default::default() })
        })
        .await
    }

    async fn deny_authorization(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DenyAuthorizationRequest>,
    ) -> ServiceResult<DenyAuthorizationResponse> {
        handle(async {
            let user_id = ctx.require_session()?.user_id;
            self.limit_consent(user_id, "view", 120).await?;
            let checked = self
                .check_authorization_request(&authorization_request(request.to_owned_message().request.as_option())?)
                .await?;
            ok(DenyAuthorizationResponse { redirect_url: checked.denied_redirect(), ..Default::default() })
        })
        .await
    }

    async fn get_device_consent(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetDeviceConsentRequest>,
    ) -> ServiceResult<GetDeviceConsentResponse> {
        handle(async {
            let user_id = ctx.require_session()?.user_id;
            ok(GetDeviceConsentResponse {
                consent: self.device_consent(user_id, request.user_code).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn approve_device(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ApproveDeviceRequest>,
    ) -> ServiceResult<ApproveDeviceResponse> {
        handle(async {
            let user_id = ctx.require_session()?.user_id;
            self.answer_device(user_id, request.user_code, true).await?;
            self.identity
                .log_security_event(
                    user_id,
                    "APP_AUTHORIZED",
                    &ctx.client(),
                    "",
                    serde_json::json!({ "device_flow": true }),
                )
                .await;
            ok(ApproveDeviceResponse::default())
        })
        .await
    }

    async fn deny_device(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DenyDeviceRequest>,
    ) -> ServiceResult<DenyDeviceResponse> {
        handle(async {
            let user_id = ctx.require_session()?.user_id;
            self.answer_device(user_id, request.user_code, false).await?;
            ok(DenyDeviceResponse::default())
        })
        .await
    }

    async fn list_authorized_apps(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListAuthorizedAppsRequest>,
    ) -> ServiceResult<ListAuthorizedAppsResponse> {
        handle(async {
            let user_id = ctx.require_session()?.user_id;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE_SIZE);
            let after: Option<Uuid> = page.decode()?;
            let mut rows: Vec<(Uuid, Uuid, Vec<String>, Time, Option<Time>)> = sqlx::query_as(
                "SELECT id, app_id, scopes, created_at, last_used_at FROM dev_authorizations
                 WHERE user_id = $1 AND ($2::uuid IS NULL OR id < $2) ORDER BY id DESC LIMIT $3",
            )
            .bind(user_id)
            .bind(after)
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
            let info = page.finish(&mut rows, |row| row.0);
            let app_ids: Vec<Uuid> = rows.iter().map(|row| row.1).collect();
            let mut apps = self.public_apps(&app_ids).await?;
            let apps = rows
                .into_iter()
                .filter_map(|(_, app_id, granted, authorized_at, last_used_at)| {
                    Some(AuthorizedApp {
                        app: apps.remove(&app_id)?.into(),
                        scopes: scopes::describe(&granted),
                        authorized_at: ts(authorized_at),
                        last_used_at: opt_ts(last_used_at),
                        ..Default::default()
                    })
                })
                .collect();
            ok(ListAuthorizedAppsResponse { apps, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn revoke_authorized_app(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RevokeAuthorizedAppRequest>,
    ) -> ServiceResult<RevokeAuthorizedAppResponse> {
        handle(async {
            let user_id = ctx.require_session()?.user_id;
            let app_id = parse_id(request.app_id)?;
            let revoked = self.revoke_authorization(app_id, user_id, "user", Some(user_id)).await?;
            ensure!(revoked, not_found, "AUTHORIZATION_NOT_FOUND", "this application has no access to your account");
            self.identity
                .log_security_event(
                    user_id,
                    "APP_ACCESS_REVOKED",
                    &ctx.client(),
                    "",
                    serde_json::json!({ "app_id": app_id }),
                )
                .await;
            ok(RevokeAuthorizedAppResponse::default())
        })
        .await
    }
}
