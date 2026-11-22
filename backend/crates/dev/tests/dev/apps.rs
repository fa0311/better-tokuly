//! Registering OAuth applications: the developer's side.
use tokuly_dev::{DevConfig, secrets};
use tokuly_proto::dev::*;

use crate::common::*;

#[tokio::test]
async fn a_confidential_client_gets_its_secret_once() {
    let ctx = start().await;
    let (dana, eve) = (ctx.app.user("dana").await, ctx.app.user("eve").await);
    let registered =
        create_app(&ctx, &dana, &["notes:read", "identity:read"], AppClientType::APP_CLIENT_TYPE_CONFIDENTIAL).await;
    assert!(secrets::is(&registered.secret, secrets::SecretKind::ClientSecret));
    let app = &registered.app;
    assert_eq!(app.client_secret_hint, format!("tkl_cs_…{}", &registered.secret[registered.secret.len() - 4..]));
    assert_eq!(
        (app.name.as_str(), app.status.as_known(), app.user_count),
        ("Test App", Some(AppStatus::APP_STATUS_ACTIVE), 0)
    );
    assert_eq!(app.redirect_uris, [REDIRECT_URI]);
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM dev_apps p WHERE to_jsonb(p)::text LIKE '%' || $1 || '%'")
            .bind(&registered.secret)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!(stored, 0, "the client secret is not in the database");

    // The owner reads it back without the secret; nobody else reads it at all.
    let get = GetAppRequest { app_id: registered.id.clone(), ..Default::default() };
    let got: GetAppResponse = ctx.app.call(&format!("{APP_SERVICE}/GetApp"), Some(&dana), &get).await.unwrap();
    assert_eq!(got.app.as_option().unwrap().id, registered.id);
    let hidden =
        ctx.app.call::<_, GetAppResponse>(&format!("{APP_SERVICE}/GetApp"), Some(&eve), &get).await.unwrap_err();
    assert_eq!((hidden.code.as_str(), hidden.reason.as_str()), ("not_found", "APP_NOT_FOUND"));
    let mine: ListAppsResponse =
        ctx.app.call(&format!("{APP_SERVICE}/ListApps"), Some(&dana), &ListAppsRequest::default()).await.unwrap();
    assert_eq!(mine.apps.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), [registered.id.as_str()]);
    let theirs: ListAppsResponse =
        ctx.app.call(&format!("{APP_SERVICE}/ListApps"), Some(&eve), &ListAppsRequest::default()).await.unwrap();
    assert!(theirs.apps.is_empty());

    // Everyone sees the public card, with who is behind it.
    let public: GetPublicAppResponse = ctx
        .app
        .call(
            &format!("{AUTHORIZATION_SERVICE}/GetPublicApp"),
            None,
            &GetPublicAppRequest { app_id: registered.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let card = public.app.as_option().unwrap();
    assert_eq!(
        (card.name.as_str(), card.owner.as_option().unwrap().handle.as_str()),
        ("Test App", dana.handle.as_str())
    );
    assert_eq!(ctx.app.events_by("dev.app.created", dana.id).await.len(), 1);

    // A public client has no secret to give.
    let public_client = create_app(&ctx, &dana, &["notes:read"], AppClientType::APP_CLIENT_TYPE_PUBLIC).await;
    assert!(public_client.secret.is_empty() && public_client.app.client_secret_hint.is_empty());
}

#[tokio::test]
async fn application_requests_are_validated() {
    let ctx = start_with(DevConfig { max_apps: 1, ..config() }).await;
    let dana = ctx.app.user("dana").await;
    let create = |request: CreateAppRequest| {
        let (ctx, dana) = (&ctx, &dana);
        async move { ctx.app.call::<_, CreateAppResponse>(&format!("{APP_SERVICE}/CreateApp"), Some(dana), &request).await }
    };
    let confidential = AppClientType::APP_CLIENT_TYPE_CONFIDENTIAL;
    let good = || app_request("Good", &["notes:read"], confidential);
    let uris = |uris: &[&str]| uris.iter().map(|u| u.to_string()).collect::<Vec<_>>();
    for (request, reason) in [
        (CreateAppRequest { name: " ".into(), ..good() }, "APP_NAME_INVALID"),
        (CreateAppRequest { description: "d".repeat(501), ..good() }, "DESCRIPTION_TOO_LONG"),
        (CreateAppRequest { homepage_url: "http://insecure.example".into(), ..good() }, "URL_INVALID"),
        (CreateAppRequest { privacy_policy_url: "nonsense".into(), ..good() }, "URL_INVALID"),
        (CreateAppRequest { redirect_uris: vec![], ..good() }, "REDIRECT_URI_REQUIRED"),
        (CreateAppRequest { redirect_uris: uris(&["http://app.example/cb"]), ..good() }, "REDIRECT_URI_INVALID"),
        (CreateAppRequest { redirect_uris: uris(&["https://app.example/cb#frag"]), ..good() }, "REDIRECT_URI_INVALID"),
        (CreateAppRequest { redirect_uris: uris(&["https://app.example/*"; 11]), ..good() }, "TOO_MANY_REDIRECT_URIS"),
        // An app scheme cannot be registered for a client that claims to keep a secret.
        (CreateAppRequest { redirect_uris: uris(&["com.example.app:/cb"]), ..good() }, "REDIRECT_URI_INVALID"),
        (CreateAppRequest { scopes: vec![], ..good() }, "SCOPES_REQUIRED"),
        (CreateAppRequest { scopes: vec!["admin:write".into()], ..good() }, "SCOPE_UNKNOWN"),
        (
            CreateAppRequest { client_type: AppClientType::APP_CLIENT_TYPE_UNSPECIFIED.into(), ..good() },
            "CLIENT_TYPE_REQUIRED",
        ),
    ] {
        assert_eq!(create(request).await.unwrap_err().reason, reason);
    }
    let anonymous =
        ctx.app.call::<_, CreateAppResponse>(&format!("{APP_SERVICE}/CreateApp"), None, &good()).await.unwrap_err();
    assert_eq!(anonymous.reason, "SIGN_IN_REQUIRED");

    // A native app: private-use scheme and loopback, as a public client.
    let native = CreateAppRequest {
        redirect_uris: uris(&["com.example.app:/cb", "http://127.0.0.1/cb"]),
        ..app_request("Native", &["notes:read"], AppClientType::APP_CLIENT_TYPE_PUBLIC)
    };
    assert!(create(native).await.is_ok());
    assert_eq!(create(good()).await.unwrap_err().reason, "TOO_MANY_APPS");
}

#[tokio::test]
async fn an_application_is_edited_and_deleted_by_its_owner_only() {
    let ctx = start().await;
    let (dana, eve) = (ctx.app.user("dana").await, ctx.app.user("eve").await);
    let registered = create_app(&ctx, &dana, &["notes:read"], AppClientType::APP_CLIENT_TYPE_CONFIDENTIAL).await;
    let icon = ctx.images.upload(dana.id);
    let update = UpdateAppRequest {
        app_id: registered.id.clone(),
        name: "Renamed".into(),
        description: "Better".into(),
        homepage_url: "https://renamed.example".into(),
        redirect_uris: vec!["https://renamed.example/cb".into(), "https://renamed.example/cb2".into()],
        scopes: vec!["notes:write".into(), "geo:read".into()],
        icon_asset_id: Some(icon.to_string()),
        ..Default::default()
    };
    let hidden = ctx
        .app
        .call::<_, UpdateAppResponse>(&format!("{APP_SERVICE}/UpdateApp"), Some(&eve), &update)
        .await
        .unwrap_err();
    assert_eq!(hidden.reason, "APP_NOT_FOUND");
    let updated: UpdateAppResponse =
        ctx.app.call(&format!("{APP_SERVICE}/UpdateApp"), Some(&dana), &update).await.unwrap();
    let app = updated.app.as_option().unwrap();
    assert_eq!(
        (app.name.as_str(), app.redirect_uris.len(), app.scopes.as_slice()),
        ("Renamed", 2, ["geo:read".to_owned(), "notes:write".to_owned()].as_slice())
    );
    assert_eq!(app.icon.as_option().unwrap().url, format!("https://media.test/{icon}.webp"));
    assert_eq!(
        ctx.images.held_by(&format!("dev:app-icon:{}", registered.id)),
        Some(icon),
        "the icon is held for the application"
    );

    // Somebody else's upload is not an icon; an unset icon is kept, an empty one removed.
    let foreign = UpdateAppRequest { icon_asset_id: Some(ctx.images.upload(eve.id).to_string()), ..update.clone() };
    assert_eq!(
        ctx.app
            .call::<_, UpdateAppResponse>(&format!("{APP_SERVICE}/UpdateApp"), Some(&dana), &foreign)
            .await
            .unwrap_err()
            .reason,
        "ASSET_NOT_FOUND"
    );
    let kept: UpdateAppResponse = ctx
        .app
        .call(
            &format!("{APP_SERVICE}/UpdateApp"),
            Some(&dana),
            &UpdateAppRequest { icon_asset_id: None, ..update.clone() },
        )
        .await
        .unwrap();
    assert!(kept.app.as_option().unwrap().icon.as_option().is_some());
    let cleared: UpdateAppResponse = ctx
        .app
        .call(
            &format!("{APP_SERVICE}/UpdateApp"),
            Some(&dana),
            &UpdateAppRequest { icon_asset_id: Some(String::new()), ..update.clone() },
        )
        .await
        .unwrap();
    assert!(cleared.app.as_option().unwrap().icon.as_option().is_none());
    assert_eq!(ctx.images.held_by(&format!("dev:app-icon:{}", registered.id)), None);
    let invalid = UpdateAppRequest { redirect_uris: vec![], ..update.clone() };
    assert_eq!(
        ctx.app
            .call::<_, UpdateAppResponse>(&format!("{APP_SERVICE}/UpdateApp"), Some(&dana), &invalid)
            .await
            .unwrap_err()
            .reason,
        "REDIRECT_URI_REQUIRED"
    );

    let delete = DeleteAppRequest { app_id: registered.id.clone(), ..Default::default() };
    assert_eq!(
        ctx.app
            .call::<_, DeleteAppResponse>(&format!("{APP_SERVICE}/DeleteApp"), Some(&eve), &delete)
            .await
            .unwrap_err()
            .reason,
        "APP_NOT_FOUND"
    );
    ctx.app.call::<_, DeleteAppResponse>(&format!("{APP_SERVICE}/DeleteApp"), Some(&dana), &delete).await.unwrap();
    assert_eq!(
        ctx.app
            .call::<_, DeleteAppResponse>(&format!("{APP_SERVICE}/DeleteApp"), Some(&dana), &delete)
            .await
            .unwrap_err()
            .reason,
        "APP_NOT_FOUND"
    );
    assert_eq!(ctx.app.events_by("dev.app.deleted", dana.id).await.len(), 1);
}

#[tokio::test]
async fn icons_need_the_media_bridge() {
    let ctx = start().await;
    let dana = ctx.app.user("dana").await;
    // A deployment whose server has not connected the media domain.
    let bare = tokuly_dev::Dev::new(ctx.app.core.clone(), ctx.app.identity.clone(), config()).unwrap();
    let router =
        axum::Router::new().fallback_service(bare.register(connectrpc::Router::new()).into_axum_service()).layer(
            axum::middleware::from_fn_with_state(ctx.app.core.auth_layer.clone(), tokuly_core::auth::auth_middleware),
        );
    let request = CreateAppRequest {
        icon_asset_id: uuid::Uuid::now_v7().to_string(),
        ..app_request("With icon", &["notes:read"], AppClientType::APP_CLIENT_TYPE_PUBLIC)
    };
    let http_request = http::Request::builder()
        .method("POST")
        .uri(format!("/{APP_SERVICE}/CreateApp"))
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", dana.token))
        .body(axum::body::Body::from(serde_json::to_vec(&request).unwrap()))
        .unwrap();
    let response = tower::ServiceExt::oneshot(router, http_request).await.unwrap();
    assert_eq!(response.status(), 501);
    assert_eq!(response.headers()["x-tokuly-error"], "ICONS_UNAVAILABLE");
    // Nothing was registered half-way.
    let apps: i64 = sqlx::query_scalar("SELECT count(*) FROM dev_apps WHERE owner_id = $1")
        .bind(dana.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(apps, 0);
}

#[tokio::test]
async fn a_rotated_client_secret_overlaps_unless_revoked() {
    let ctx = start().await;
    let (dana, eve, user) = (ctx.app.user("dana").await, ctx.app.user("eve").await, ctx.app.user("user").await);
    let mut app = create_app(&ctx, &dana, &["notes:read"], AppClientType::APP_CLIENT_TYPE_CONFIDENTIAL).await;
    let original = app.secret.clone();
    let app_id = app.id.clone();
    let rotate = |revoke_previous: bool, caller: &tokuly_testkit::TestUser| {
        let (ctx, caller, request) = (
            &ctx,
            caller.clone(),
            RotateClientSecretRequest { app_id: app_id.clone(), revoke_previous, ..Default::default() },
        );
        async move {
            ctx.app
                .call::<_, RotateClientSecretResponse>(
                    &format!("{APP_SERVICE}/RotateClientSecret"),
                    Some(&caller),
                    &request,
                )
                .await
        }
    };
    assert_eq!(rotate(false, &eve).await.unwrap_err().reason, "APP_NOT_FOUND");
    let rotated = rotate(false, &dana).await.unwrap();
    assert!(
        rotated.client_secret != original && secrets::is(&rotated.client_secret, secrets::SecretKind::ClientSecret)
    );
    assert!(rotated.app.as_option().unwrap().previous_secret_expires_at.as_option().is_some());

    // A deployment still holding the old secret keeps working for the overlap; so does the new one.
    let (code, verifier) = authorize(&ctx, &user, &app, "notes:read").await;
    assert_eq!(exchange(&ctx, &app, &code, &verifier).await.status, 200);
    app.secret = rotated.client_secret.clone();
    let (code, verifier) = authorize(&ctx, &user, &app, "notes:read").await;
    assert_eq!(exchange(&ctx, &app, &code, &verifier).await.status, 200);

    // A leaked secret is cut off at once: neither it nor the one before it works.
    let emergency = rotate(true, &dana).await.unwrap();
    assert!(emergency.app.as_option().unwrap().previous_secret_expires_at.as_option().is_none());
    for dead in [original.as_str(), rotated.client_secret.as_str()] {
        app.secret = dead.to_owned();
        let (code, verifier) = authorize(&ctx, &user, &app, "notes:read").await;
        let reply = exchange(&ctx, &app, &code, &verifier).await;
        assert_eq!((reply.status, reply.error().as_str()), (401, "invalid_client"));
    }
    app.secret = emergency.client_secret;
    let (code, verifier) = authorize(&ctx, &user, &app, "notes:read").await;
    assert_eq!(exchange(&ctx, &app, &code, &verifier).await.status, 200);
    assert_eq!(ctx.security_events(dana.id, "APP_SECRET_ROTATED").await, 2);

    let public = create_app(&ctx, &dana, &["notes:read"], AppClientType::APP_CLIENT_TYPE_PUBLIC).await;
    let request = RotateClientSecretRequest { app_id: public.id, ..Default::default() };
    let refused = ctx
        .app
        .call::<_, RotateClientSecretResponse>(&format!("{APP_SERVICE}/RotateClientSecret"), Some(&dana), &request)
        .await
        .unwrap_err();
    assert_eq!(refused.reason, "CLIENT_IS_PUBLIC");
}

#[tokio::test]
async fn the_owner_sees_who_uses_the_application() {
    let ctx = start().await;
    let (dana, eve) = (ctx.app.user("dana").await, ctx.app.user("eve").await);
    let (ann, ben) = (ctx.app.user("ann").await, ctx.app.user("ben").await);
    let app =
        create_app(&ctx, &dana, &["identity:read", "notes:read"], AppClientType::APP_CLIENT_TYPE_CONFIDENTIAL).await;
    let (ann_access, _) = obtain_tokens(&ctx, &ann, &app, "identity:read").await;
    obtain_tokens(&ctx, &ben, &app, "notes:read").await;
    // Ann's token is used; Ben's never is.
    assert!(whoami(&ctx, &with_token(&ann, &ann_access)).await.is_ok());

    let request = ListAppUsersRequest { app_id: app.id.clone(), ..Default::default() };
    eventually("ann's activity is recorded", || async {
        let listed: ListAppUsersResponse =
            ctx.app.call(&format!("{APP_SERVICE}/ListAppUsers"), Some(&dana), &request).await.unwrap();
        listed.users.first().is_some_and(|first| first.last_used_at.as_option().is_some())
    })
    .await;
    let listed: ListAppUsersResponse =
        ctx.app.call(&format!("{APP_SERVICE}/ListAppUsers"), Some(&dana), &request).await.unwrap();
    let users: Vec<(&str, &[String], bool)> = listed
        .users
        .iter()
        .map(|u| {
            (u.user.as_option().unwrap().handle.as_str(), u.scopes.as_slice(), u.last_used_at.as_option().is_some())
        })
        .collect();
    assert_eq!(
        users,
        [
            (ann.handle.as_str(), ["identity:read".to_owned()].as_slice(), true),
            (ben.handle.as_str(), ["notes:read".to_owned()].as_slice(), false)
        ]
    );
    // Paged, most recently active first.
    let first_page = ListAppUsersRequest {
        page: tokuly_proto::common::PageRequest { limit: 1, ..Default::default() }.into(),
        ..request.clone()
    };
    let first: ListAppUsersResponse =
        ctx.app.call(&format!("{APP_SERVICE}/ListAppUsers"), Some(&dana), &first_page).await.unwrap();
    let page = first.page.as_option().unwrap();
    assert!(first.users.len() == 1 && page.has_more);
    let second_page = ListAppUsersRequest {
        page: tokuly_proto::common::PageRequest { limit: 1, cursor: page.next_cursor.clone(), ..Default::default() }
            .into(),
        ..request.clone()
    };
    let second: ListAppUsersResponse =
        ctx.app.call(&format!("{APP_SERVICE}/ListAppUsers"), Some(&dana), &second_page).await.unwrap();
    assert_eq!(second.users[0].user.as_option().unwrap().handle, ben.handle);
    assert!(!second.page.as_option().unwrap().has_more);

    let got: GetAppResponse = ctx
        .app
        .call(
            &format!("{APP_SERVICE}/GetApp"),
            Some(&dana),
            &GetAppRequest { app_id: app.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(got.app.as_option().unwrap().user_count, 2);
    assert_eq!(
        ctx.app
            .call::<_, ListAppUsersResponse>(&format!("{APP_SERVICE}/ListAppUsers"), Some(&eve), &request)
            .await
            .unwrap_err()
            .reason,
        "APP_NOT_FOUND"
    );

    // Usage of the application, all users together.
    let usage_request =
        GetUsageRequest { subject: Some(get_usage_request::Subject::AppId(app.id.clone())), ..Default::default() };
    eventually("the application's usage reaches the analytics store", || async {
        ctx.app.flush_analytics().await;
        ctx.app
            .call::<_, GetUsageResponse>("tokuly.dev.v1.UsageService/GetUsage", Some(&dana), &usage_request)
            .await
            .is_ok_and(|usage| usage.requests == 1)
    })
    .await;
    assert_eq!(
        ctx.app
            .call::<_, GetUsageResponse>("tokuly.dev.v1.UsageService/GetUsage", Some(&eve), &usage_request)
            .await
            .unwrap_err()
            .reason,
        "APP_NOT_FOUND"
    );
}
