//! OAuth 2.1 as a client library sees it: the consent RPCs, the token,
//! revocation and device endpoints, and the errors the RFCs prescribe.
use tokuly_dev::{DevConfig, secrets};
use tokuly_proto::{dev::*, identity};

use crate::common::*;

const CONFIDENTIAL: AppClientType = AppClientType::APP_CLIENT_TYPE_CONFIDENTIAL;
const PUBLIC: AppClientType = AppClientType::APP_CLIENT_TYPE_PUBLIC;
const GET_ME: &str = "tokuly.identity.v1.AccountService/GetMe";

async fn consent(
    ctx: &Ctx,
    user: &tokuly_testkit::TestUser,
    request: AuthorizationRequest,
) -> Result<GetConsentResponse, tokuly_testkit::RpcError> {
    ctx.app
        .call(
            &format!("{AUTHORIZATION_SERVICE}/GetConsent"),
            Some(user),
            &GetConsentRequest { request: request.into(), ..Default::default() },
        )
        .await
}

/// The OAuth error an authorization request is answered with by redirect.
async fn redirected_error(
    ctx: &Ctx,
    user: &tokuly_testkit::TestUser,
    request: AuthorizationRequest,
) -> (String, String) {
    match consent(ctx, user, request).await.expect("the request is answered by redirect").result {
        Some(get_consent_response::Result::RedirectUrl(url)) => {
            assert!(url.starts_with(REDIRECT_URI), "{url}");
            let mut params = query(&url);
            (params.remove("error").unwrap_or_default(), params.remove("state").unwrap_or_default())
        }
        other => panic!("expected an error redirect, got {other:?}"),
    }
}

async fn refresh(ctx: &Ctx, app: &OAuthApp, refresh_token: &str) -> HttpReply {
    ctx.form("/oauth/token", app.basic(), &[("grant_type", "refresh_token"), ("refresh_token", refresh_token)]).await
}

#[tokio::test]
async fn the_authorization_code_flow_ends_in_a_scoped_token() {
    let ctx = start().await;
    let (dana, user) = (ctx.app.user("dana").await, ctx.app.user("user").await);
    let app = create_app(&ctx, &dana, &["identity:read", "geo:read", "notes:write"], CONFIDENTIAL).await;
    let (verifier, challenge) = pkce();
    let request = authorization_request(&app, "identity:read geo:read", &challenge);

    // The consent screen: who asks, for what, in words.
    let shown = match consent(&ctx, &user, request.clone()).await.unwrap().result {
        Some(get_consent_response::Result::Consent(consent)) => consent,
        other => panic!("expected a consent screen, got {other:?}"),
    };
    let card = shown.app.as_option().unwrap();
    assert_eq!(
        (card.name.as_str(), card.owner.as_option().unwrap().handle.as_str()),
        ("Test App", dana.handle.as_str())
    );
    assert_eq!(
        shown.scopes.iter().map(|s| s.scope.as_str()).collect::<Vec<_>>(),
        ["geo:read", "identity:read"],
        "location comes first"
    );
    assert!(shown.scopes[0].sensitive && shown.scopes[0].description.as_option().unwrap().ja.contains("現在地"));
    assert_eq!((shown.redirect_host.as_str(), shown.already_granted), ("app.example", false));

    // Approval sends the browser back with a code, the state and the issuer.
    let approve = ApproveAuthorizationRequest { request: request.clone().into(), ..Default::default() };
    let approved: ApproveAuthorizationResponse =
        ctx.app.call(&format!("{AUTHORIZATION_SERVICE}/ApproveAuthorization"), Some(&user), &approve).await.unwrap();
    assert!(approved.redirect_url.starts_with(&format!("{REDIRECT_URI}?")));
    let params = query(&approved.redirect_url);
    assert_eq!((params["state"].as_str(), params["iss"].as_str()), ("state-123", "http://localhost:8080/api"));
    assert!(secrets::is(&params["code"], secrets::SecretKind::Code));
    assert_eq!(ctx.notifications(user.id, "dev.app_authorized").await.len(), 1);
    assert_eq!(ctx.security_events(user.id, "APP_AUTHORIZED").await, 1);

    // The code buys a token pair, once.
    let reply = exchange(&ctx, &app, &params["code"], &verifier).await;
    assert_eq!((reply.status, reply.header("cache-control"), reply.header("pragma")), (200, "no-store", "no-cache"));
    let tokens = reply.json();
    assert_eq!(
        (tokens["token_type"].as_str(), tokens["expires_in"].as_i64(), tokens["scope"].as_str()),
        (Some("Bearer"), Some(3600), Some("geo:read identity:read"))
    );
    let access = tokens["access_token"].as_str().unwrap();
    assert!(
        secrets::is(access, secrets::SecretKind::Access)
            && secrets::is(tokens["refresh_token"].as_str().unwrap(), secrets::SecretKind::Refresh)
    );
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM dev_oauth_tokens o WHERE to_jsonb(o)::text LIKE '%' || $1 || '%' OR to_jsonb(o)::text LIKE '%' || $2 || '%'")
        .bind(access)
        .bind(tokens["refresh_token"].as_str().unwrap())
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(stored, 0, "tokens are stored hashed");

    // The access token is the user, limited to what was granted, and knows its application.
    let caller = with_token(&user, access);
    let me = whoami(&ctx, &caller).await.unwrap();
    assert_eq!(me.kind.as_known(), Some(CredentialKind::CREDENTIAL_KIND_APP));
    assert_eq!(
        (me.user.as_option().unwrap().id.as_str(), me.app.as_option().unwrap().id.as_str()),
        (user.id.to_string().as_str(), app.id.as_str())
    );
    assert_eq!(me.scopes, ["geo:read", "identity:read"]);
    assert!(
        ctx.app
            .call::<_, identity::GetMeResponse>(GET_ME, Some(&caller), &identity::GetMeRequest::default())
            .await
            .is_ok()
    );
    let update = identity::UpdateProfileRequest { bio: Some("by an app".into()), ..Default::default() };
    let denied = ctx
        .app
        .call::<_, identity::UpdateProfileResponse>(
            "tokuly.identity.v1.AccountService/UpdateProfile",
            Some(&caller),
            &update,
        )
        .await
        .unwrap_err();
    assert_eq!(denied.reason, "INSUFFICIENT_SCOPE");
    // An application cannot give itself more, nor manage the user's tokens or consent.
    let denied = ctx
        .app
        .call::<_, ListAuthorizedAppsResponse>(
            &format!("{AUTHORIZATION_SERVICE}/ListAuthorizedApps"),
            Some(&caller),
            &ListAuthorizedAppsRequest::default(),
        )
        .await;
    assert_eq!(denied.unwrap_err().reason, "SESSION_REQUIRED");

    // Asked again for the same, the screen can say so.
    match consent(&ctx, &user, request).await.unwrap().result {
        Some(get_consent_response::Result::Consent(consent)) => assert!(consent.already_granted),
        other => panic!("expected a consent screen, got {other:?}"),
    }
}

#[tokio::test]
async fn requests_that_cannot_be_trusted_are_shown_not_redirected() {
    let ctx = start().await;
    let (dana, user) = (ctx.app.user("dana").await, ctx.app.user("user").await);
    let app = create_app(&ctx, &dana, &["identity:read"], CONFIDENTIAL).await;
    let (_, challenge) = pkce();
    let good = authorization_request(&app, "identity:read", &challenge);

    for (request, code, reason) in [
        (
            AuthorizationRequest { client_id: uuid::Uuid::now_v7().to_string(), ..good.clone() },
            "not_found",
            "CLIENT_NOT_FOUND",
        ),
        (AuthorizationRequest { client_id: "not-an-id".into(), ..good.clone() }, "not_found", "CLIENT_NOT_FOUND"),
        (
            AuthorizationRequest { redirect_uri: "https://evil.example/callback".into(), ..good.clone() },
            "invalid_argument",
            "REDIRECT_URI_MISMATCH",
        ),
        (
            AuthorizationRequest { redirect_uri: format!("{REDIRECT_URI}/extra"), ..good.clone() },
            "invalid_argument",
            "REDIRECT_URI_MISMATCH",
        ),
        (
            AuthorizationRequest { redirect_uri: format!("{REDIRECT_URI}?next=https://evil.example"), ..good.clone() },
            "invalid_argument",
            "REDIRECT_URI_MISMATCH",
        ),
    ] {
        let refused = consent(&ctx, &user, request.clone()).await.unwrap_err();
        assert_eq!((refused.code.as_str(), refused.reason.as_str()), (code, reason));
        // Neither approving nor denying produces a redirect to an unvetted place.
        let approve = ApproveAuthorizationRequest { request: request.clone().into(), ..Default::default() };
        assert_eq!(
            ctx.app
                .call::<_, ApproveAuthorizationResponse>(
                    &format!("{AUTHORIZATION_SERVICE}/ApproveAuthorization"),
                    Some(&user),
                    &approve
                )
                .await
                .unwrap_err()
                .reason,
            reason
        );
        let deny = DenyAuthorizationRequest { request: request.into(), ..Default::default() };
        assert_eq!(
            ctx.app
                .call::<_, DenyAuthorizationResponse>(
                    &format!("{AUTHORIZATION_SERVICE}/DenyAuthorization"),
                    Some(&user),
                    &deny
                )
                .await
                .unwrap_err()
                .reason,
            reason
        );
    }
    // The only registered URI may be left out.
    assert!(matches!(
        consent(&ctx, &user, AuthorizationRequest { redirect_uri: String::new(), ..good.clone() })
            .await
            .unwrap()
            .result,
        Some(get_consent_response::Result::Consent(_))
    ));
    // The consent screen is for a signed-in person.
    let anonymous = ctx
        .app
        .call::<_, GetConsentResponse>(
            &format!("{AUTHORIZATION_SERVICE}/GetConsent"),
            None,
            &GetConsentRequest { request: good.clone().into(), ..Default::default() },
        )
        .await;
    assert_eq!(anonymous.unwrap_err().reason, "SIGN_IN_REQUIRED");
    let missing = ctx
        .app
        .call::<_, GetConsentResponse>(
            &format!("{AUTHORIZATION_SERVICE}/GetConsent"),
            Some(&user),
            &GetConsentRequest::default(),
        )
        .await;
    assert_eq!(missing.unwrap_err().reason, "AUTHORIZATION_REQUEST_INVALID");
}

#[tokio::test]
async fn other_defects_are_reported_to_the_application_by_redirect() {
    let ctx = start().await;
    let (dana, user) = (ctx.app.user("dana").await, ctx.app.user("user").await);
    let app = create_app(&ctx, &dana, &["identity:read"], CONFIDENTIAL).await;
    let (_, challenge) = pkce();
    let good = authorization_request(&app, "identity:read", &challenge);
    for (request, error) in [
        (AuthorizationRequest { response_type: "token".into(), ..good.clone() }, "unsupported_response_type"),
        (AuthorizationRequest { response_type: String::new(), ..good.clone() }, "unsupported_response_type"),
        // PKCE is required of every client, and only S256.
        (AuthorizationRequest { code_challenge: String::new(), ..good.clone() }, "invalid_request"),
        (AuthorizationRequest { code_challenge_method: "plain".into(), ..good.clone() }, "invalid_request"),
        (AuthorizationRequest { code_challenge_method: String::new(), ..good.clone() }, "invalid_request"),
        (AuthorizationRequest { code_challenge: "too-short".into(), ..good.clone() }, "invalid_request"),
        // Only what the application registered, and only real scopes.
        (AuthorizationRequest { scope: "identity:read chat:read".into(), ..good.clone() }, "invalid_scope"),
        (AuthorizationRequest { scope: "identity:write".into(), ..good.clone() }, "invalid_scope"),
        (AuthorizationRequest { scope: "everything".into(), ..good.clone() }, "invalid_scope"),
    ] {
        assert_eq!(
            redirected_error(&ctx, &user, request.clone()).await,
            (error.to_owned(), "state-123".to_owned()),
            "{request:?}"
        );
        // Such a request cannot be approved into a code.
        let approve = ApproveAuthorizationRequest { request: request.into(), ..Default::default() };
        let refused = ctx
            .app
            .call::<_, ApproveAuthorizationResponse>(
                &format!("{AUTHORIZATION_SERVICE}/ApproveAuthorization"),
                Some(&user),
                &approve,
            )
            .await
            .unwrap_err();
        assert_eq!(refused.reason, "AUTHORIZATION_REQUEST_INVALID");
    }
    // No scope asks for everything registered.
    match consent(&ctx, &user, AuthorizationRequest { scope: String::new(), ..good.clone() }).await.unwrap().result {
        Some(get_consent_response::Result::Consent(consent)) => assert_eq!(consent.scopes.len(), 1),
        other => panic!("expected a consent screen, got {other:?}"),
    }
    // The user says no: the application hears `access_denied`, and nothing is granted.
    let deny = DenyAuthorizationRequest { request: good.into(), ..Default::default() };
    let denied: DenyAuthorizationResponse =
        ctx.app.call(&format!("{AUTHORIZATION_SERVICE}/DenyAuthorization"), Some(&user), &deny).await.unwrap();
    let params = query(&denied.redirect_url);
    assert_eq!(
        (params["error"].as_str(), params["state"].as_str(), params.contains_key("code")),
        ("access_denied", "state-123", false)
    );
    let listed: ListAuthorizedAppsResponse = ctx
        .app
        .call(
            &format!("{AUTHORIZATION_SERVICE}/ListAuthorizedApps"),
            Some(&user),
            &ListAuthorizedAppsRequest::default(),
        )
        .await
        .unwrap();
    assert!(listed.apps.is_empty());
}

#[tokio::test]
async fn a_code_is_bound_to_its_verifier_client_and_single_use() {
    let ctx = start().await;
    let (dana, user) = (ctx.app.user("dana").await, ctx.app.user("user").await);
    let app = create_app(&ctx, &dana, &["identity:read"], CONFIDENTIAL).await;
    let other = create_app(&ctx, &dana, &["identity:read"], CONFIDENTIAL).await;
    let token = |code: String, extra: Vec<(&'static str, String)>| {
        let (ctx, app) = (&ctx, &app);
        async move {
            let mut params = vec![("grant_type", "authorization_code".to_owned()), ("code", code)];
            params.extend(extra);
            let borrowed: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
            ctx.form("/oauth/token", app.basic(), &borrowed).await
        }
    };
    let error = |reply: HttpReply| (reply.status, reply.error());

    // The wrong verifier, and then even the right one: the code is spent by the first try.
    let (code, verifier) = authorize(&ctx, &user, &app, "identity:read").await;
    let (wrong, _) = pkce();
    assert_eq!(error(token(code.clone(), vec![("code_verifier", wrong)]).await), (400, "invalid_grant".to_owned()));
    assert_eq!(error(token(code, vec![("code_verifier", verifier)]).await), (400, "invalid_grant".to_owned()));

    // No verifier at all, or one that is not a verifier.
    let (code, _) = authorize(&ctx, &user, &app, "identity:read").await;
    assert_eq!(error(token(code, vec![]).await), (400, "invalid_request".to_owned()));
    let (code, _) = authorize(&ctx, &user, &app, "identity:read").await;
    assert_eq!(
        error(token(code, vec![("code_verifier", "short".to_owned())]).await),
        (400, "invalid_request".to_owned())
    );

    // A redirect_uri, when sent, must be the one the code went to.
    let (code, verifier) = authorize(&ctx, &user, &app, "identity:read").await;
    assert_eq!(
        error(
            token(code, vec![("code_verifier", verifier), ("redirect_uri", "https://evil.example/cb".to_owned())])
                .await
        ),
        (400, "invalid_grant".to_owned())
    );
    let (code, verifier) = authorize(&ctx, &user, &app, "identity:read").await;
    assert_eq!(
        token(code, vec![("code_verifier", verifier), ("redirect_uri", REDIRECT_URI.to_owned())]).await.status,
        200
    );

    // Another client cannot redeem it.
    let (code, verifier) = authorize(&ctx, &user, &app, "identity:read").await;
    let stolen = ctx
        .form(
            "/oauth/token",
            other.basic(),
            &[("grant_type", "authorization_code"), ("code", &code), ("code_verifier", &verifier)],
        )
        .await;
    assert_eq!(error(stolen), (400, "invalid_grant".to_owned()));

    // A code used twice was intercepted: the tokens it bought are revoked.
    let (code, verifier) = authorize(&ctx, &user, &app, "identity:read").await;
    let first = exchange(&ctx, &app, &code, &verifier).await.json();
    let access = first["access_token"].as_str().unwrap().to_owned();
    assert!(whoami(&ctx, &with_token(&user, &access)).await.is_ok());
    assert_eq!(error(exchange(&ctx, &app, &code, &verifier).await), (400, "invalid_grant".to_owned()));
    assert_eq!(whoami(&ctx, &with_token(&user, &access)).await.unwrap_err().reason, "TOKEN_REVOKED");
    assert_eq!(
        error(refresh(&ctx, &app, first["refresh_token"].as_str().unwrap()).await),
        (400, "invalid_grant".to_owned())
    );

    assert_eq!(
        error(token("tkl_oac_garbage".to_owned(), vec![("code_verifier", pkce().0)]).await),
        (400, "invalid_grant".to_owned())
    );
}

#[tokio::test]
async fn clients_authenticate_at_the_token_endpoint() {
    let ctx = start().await;
    let (dana, user) = (ctx.app.user("dana").await, ctx.app.user("user").await);
    let app = create_app(&ctx, &dana, &["identity:read"], CONFIDENTIAL).await;
    let grant = |code: &str, verifier: &str| {
        vec![
            ("grant_type", "authorization_code".to_owned()),
            ("code", code.to_owned()),
            ("code_verifier", verifier.to_owned()),
        ]
    };
    let send = |basic: Option<(String, String)>, params: Vec<(&'static str, String)>| {
        let ctx = &ctx;
        async move {
            let borrowed: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
            ctx.form("/oauth/token", basic.as_ref().map(|(id, secret)| (id.as_str(), secret.as_str())), &borrowed).await
        }
    };
    let (code, verifier) = authorize(&ctx, &user, &app, "identity:read").await;

    // A wrong secret: 401, answering the scheme that was used.
    let wrong =
        send(Some((app.id.clone(), secrets::generate(secrets::SecretKind::ClientSecret))), grant(&code, &verifier))
            .await;
    assert_eq!(
        (wrong.status, wrong.error().as_str(), wrong.header("www-authenticate")),
        (401, "invalid_client", r#"Basic realm="tokuly""#)
    );
    // No credentials at all for a confidential client; an unknown client.
    let mut bare = grant(&code, &verifier);
    bare.push(("client_id", app.id.clone()));
    let unauthenticated = send(None, bare.clone()).await;
    assert_eq!(
        (unauthenticated.status, unauthenticated.error().as_str(), unauthenticated.header("www-authenticate")),
        (401, "invalid_client", "")
    );
    assert_eq!(send(None, grant(&code, &verifier)).await.error(), "invalid_client");
    assert_eq!(
        send(Some((uuid::Uuid::now_v7().to_string(), app.secret.clone())), grant(&code, &verifier)).await.error(),
        "invalid_client"
    );
    // Two authentication methods at once, or a repeated parameter.
    let mut both = bare.clone();
    both.push(("client_secret", app.secret.clone()));
    assert_eq!(send(Some((app.id.clone(), app.secret.clone())), both.clone()).await.error(), "invalid_request");
    let mut repeated = grant(&code, &verifier);
    repeated.push(("code", code.clone()));
    assert_eq!(send(Some((app.id.clone(), app.secret.clone())), repeated).await.error(), "invalid_request");
    // Grants that do not exist here.
    for grant_type in ["password", "client_credentials", "implicit"] {
        let reply = send(Some((app.id.clone(), app.secret.clone())), vec![("grant_type", grant_type.to_owned())]).await;
        assert_eq!((reply.status, reply.error().as_str()), (400, "unsupported_grant_type"), "{grant_type}");
    }
    assert_eq!(send(Some((app.id.clone(), app.secret.clone())), vec![]).await.error(), "invalid_request");
    // None of the failed attempts spent the code: the secret in the body works too.
    let posted = send(None, both).await;
    assert_eq!(posted.status, 200, "{}", String::from_utf8_lossy(&posted.body));

    // A public client proves itself with PKCE alone, and must not send a secret.
    let native = create_app(&ctx, &dana, &["identity:read"], PUBLIC).await;
    let (code, verifier) = authorize(&ctx, &user, &native, "identity:read").await;
    let mut with_secret = grant(&code, &verifier);
    with_secret.extend([("client_id", native.id.clone()), ("client_secret", app.secret.clone())]);
    assert_eq!(send(None, with_secret).await.error(), "invalid_client");
    let mut public = grant(&code, &verifier);
    public.push(("client_id", native.id.clone()));
    let tokens = send(None, public).await;
    assert_eq!(tokens.status, 200);
    assert!(whoami(&ctx, &with_token(&user, tokens.json()["access_token"].as_str().unwrap())).await.is_ok());
}

#[tokio::test]
async fn refresh_tokens_rotate_and_a_replayed_one_revokes_the_family() {
    let ctx = start().await;
    let (dana, user) = (ctx.app.user("dana").await, ctx.app.user("user").await);
    let app = create_app(&ctx, &dana, &["identity:read", "notes:read"], CONFIDENTIAL).await;
    let other = create_app(&ctx, &dana, &["identity:read"], CONFIDENTIAL).await;
    let (first_access, first_refresh) = obtain_tokens(&ctx, &user, &app, "identity:read notes:read").await;

    // Another client cannot use it (and does not burn it).
    let stolen = refresh(&ctx, &other, &first_refresh).await;
    assert_eq!((stolen.status, stolen.error().as_str()), (400, "invalid_grant"));
    // Scopes can be narrowed, never widened.
    let wider = ctx
        .form(
            "/oauth/token",
            app.basic(),
            &[("grant_type", "refresh_token"), ("refresh_token", &first_refresh), ("scope", "identity:read geo:read")],
        )
        .await;
    assert_eq!(wider.error(), "invalid_scope");

    // Each use hands out a new pair and retires the old refresh token.
    let second = refresh(&ctx, &app, &first_refresh).await;
    assert_eq!(second.status, 200, "{}", String::from_utf8_lossy(&second.body));
    let second = second.json();
    let (second_access, second_refresh) =
        (second["access_token"].as_str().unwrap().to_owned(), second["refresh_token"].as_str().unwrap().to_owned());
    assert!(second_refresh != first_refresh && second_access != first_access);
    assert_eq!(second["scope"], "identity:read notes:read");
    assert!(whoami(&ctx, &with_token(&user, &second_access)).await.is_ok());
    let narrowed = ctx
        .form(
            "/oauth/token",
            app.basic(),
            &[("grant_type", "refresh_token"), ("refresh_token", &second_refresh), ("scope", "identity:read")],
        )
        .await
        .json();
    assert_eq!(narrowed["scope"], "identity:read");
    let third_access = narrowed["access_token"].as_str().unwrap().to_owned();
    assert_eq!(whoami(&ctx, &with_token(&user, &third_access)).await.unwrap().scopes, ["identity:read"]);

    // The retired token shows up again: someone has a copy. Everything descended from it dies.
    let replayed = refresh(&ctx, &app, &first_refresh).await;
    assert_eq!((replayed.status, replayed.error().as_str()), (400, "invalid_grant"));
    for access in [&first_access, &second_access, &third_access] {
        assert_eq!(whoami(&ctx, &with_token(&user, access)).await.unwrap_err().reason, "TOKEN_REVOKED");
    }
    let latest = refresh(&ctx, &app, narrowed["refresh_token"].as_str().unwrap()).await;
    assert_eq!(latest.error(), "invalid_grant", "the newest refresh token of the family is dead too");
    // The authorization itself stands: the user can sign the application in again.
    let (again, _) = obtain_tokens(&ctx, &user, &app, "identity:read").await;
    assert!(whoami(&ctx, &with_token(&user, &again)).await.is_ok());

    assert_eq!(refresh(&ctx, &app, "tkl_ort_garbage").await.error(), "invalid_grant");
    assert_eq!(
        ctx.form("/oauth/token", app.basic(), &[("grant_type", "refresh_token")]).await.error(),
        "invalid_request"
    );
}

#[tokio::test]
async fn an_access_token_expires_but_its_refresh_token_renews_it() {
    let ctx = start().await;
    let (dana, user) = (ctx.app.user("dana").await, ctx.app.user("user").await);
    let app = create_app(&ctx, &dana, &["identity:read"], CONFIDENTIAL).await;
    let (access, refresh_token) = obtain_tokens(&ctx, &user, &app, "identity:read").await;
    // Only this user's tokens: the database is shared with the other tests.
    let mine = "WHERE authorization_id IN (SELECT id FROM dev_authorizations WHERE user_id = $1)";
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE dev_oauth_tokens SET access_expires_at = now() - interval '1 second' {mine}"
    )))
    .bind(user.id)
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(whoami(&ctx, &with_token(&user, &access)).await.unwrap_err().reason, "TOKEN_EXPIRED");
    let renewed = refresh(&ctx, &app, &refresh_token).await.json();
    assert!(whoami(&ctx, &with_token(&user, renewed["access_token"].as_str().unwrap())).await.is_ok());
    // A refresh token past its own lifetime is no good either.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE dev_oauth_tokens SET refresh_expires_at = now() - interval '1 second' {mine}"
    )))
    .bind(user.id)
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(refresh(&ctx, &app, renewed["refresh_token"].as_str().unwrap()).await.error(), "invalid_grant");
}

#[tokio::test]
async fn tokens_are_revoked_at_the_revocation_endpoint() {
    let ctx = start().await;
    let (dana, user) = (ctx.app.user("dana").await, ctx.app.user("user").await);
    let app = create_app(&ctx, &dana, &["identity:read"], CONFIDENTIAL).await;
    let other = create_app(&ctx, &dana, &["identity:read"], CONFIDENTIAL).await;
    let (access, refresh_token) = obtain_tokens(&ctx, &user, &app, "identity:read").await;
    assert!(whoami(&ctx, &with_token(&user, &access)).await.is_ok());

    // Unknown tokens are not an error (RFC 7009); a client must be who it says.
    let unknown =
        ctx.form("/oauth/revoke", app.basic(), &[("token", &secrets::generate(secrets::SecretKind::Refresh))]).await;
    assert_eq!((unknown.status, unknown.body.len()), (200, 0));
    assert_eq!(ctx.form("/oauth/revoke", app.basic(), &[("token", "whatever")]).await.status, 200);
    assert_eq!(ctx.form("/oauth/revoke", Some((&app.id, "wrong")), &[("token", &refresh_token)]).await.status, 401);
    assert_eq!(ctx.form("/oauth/revoke", app.basic(), &[]).await.error(), "invalid_request");
    // Another client's token: answered exactly like one that does not exist, and left alone.
    let foreign = ctx.form("/oauth/revoke", other.basic(), &[("token", &refresh_token)]).await;
    assert_eq!((foreign.status, foreign.body.len()), (unknown.status, unknown.body.len()));
    assert!(whoami(&ctx, &with_token(&user, &access)).await.is_ok());

    // Revoking the refresh token ends the access token with it, at once.
    let revoked = ctx
        .form("/oauth/revoke", app.basic(), &[("token", &refresh_token), ("token_type_hint", "refresh_token")])
        .await;
    assert_eq!(revoked.status, 200);
    assert_eq!(whoami(&ctx, &with_token(&user, &access)).await.unwrap_err().reason, "TOKEN_REVOKED");
    assert_eq!(refresh(&ctx, &app, &refresh_token).await.error(), "invalid_grant");

    // Revoking an access token.
    let (access, _) = obtain_tokens(&ctx, &user, &app, "identity:read").await;
    assert_eq!(ctx.form("/oauth/revoke", app.basic(), &[("token", &access)]).await.status, 200);
    assert_eq!(whoami(&ctx, &with_token(&user, &access)).await.unwrap_err().reason, "TOKEN_REVOKED");
}

#[tokio::test]
async fn a_user_lists_and_withdraws_what_applications_may_do() {
    let ctx = start().await;
    let (dana, user, bystander) =
        (ctx.app.user("dana").await, ctx.app.user("user").await, ctx.app.user("bystander").await);
    let app = create_app(&ctx, &dana, &["identity:read", "geo:read"], CONFIDENTIAL).await;
    let (access, refresh_token) = obtain_tokens(&ctx, &user, &app, "identity:read").await;
    // A later, wider consent adds to the same authorization.
    obtain_tokens(&ctx, &user, &app, "geo:read").await;

    let listed: ListAuthorizedAppsResponse = ctx
        .app
        .call(
            &format!("{AUTHORIZATION_SERVICE}/ListAuthorizedApps"),
            Some(&user),
            &ListAuthorizedAppsRequest::default(),
        )
        .await
        .unwrap();
    assert_eq!(listed.apps.len(), 1);
    assert_eq!(listed.apps[0].app.as_option().unwrap().id, app.id);
    assert_eq!(
        listed.apps[0].scopes.iter().map(|s| s.scope.as_str()).collect::<Vec<_>>(),
        ["geo:read", "identity:read"]
    );
    assert!(listed.apps[0].scopes[0].description.as_option().is_some(), "scopes come with their wording");
    assert_eq!(ctx.notifications(user.id, "dev.app_authorized").await.len(), 1, "told once, not on every consent");

    let revoke = RevokeAuthorizedAppRequest { app_id: app.id.clone(), ..Default::default() };
    let none = ctx
        .app
        .call::<_, RevokeAuthorizedAppResponse>(
            &format!("{AUTHORIZATION_SERVICE}/RevokeAuthorizedApp"),
            Some(&bystander),
            &revoke,
        )
        .await
        .unwrap_err();
    assert_eq!((none.code.as_str(), none.reason.as_str()), ("not_found", "AUTHORIZATION_NOT_FOUND"));
    assert!(whoami(&ctx, &with_token(&user, &access)).await.is_ok());

    ctx.app
        .call::<_, RevokeAuthorizedAppResponse>(
            &format!("{AUTHORIZATION_SERVICE}/RevokeAuthorizedApp"),
            Some(&user),
            &revoke,
        )
        .await
        .unwrap();
    assert_eq!(whoami(&ctx, &with_token(&user, &access)).await.unwrap_err().reason, "TOKEN_INVALID");
    assert_eq!(refresh(&ctx, &app, &refresh_token).await.error(), "invalid_grant");
    let listed: ListAuthorizedAppsResponse = ctx
        .app
        .call(
            &format!("{AUTHORIZATION_SERVICE}/ListAuthorizedApps"),
            Some(&user),
            &ListAuthorizedAppsRequest::default(),
        )
        .await
        .unwrap();
    assert!(listed.apps.is_empty());
    let events = ctx.app.events_by("dev.authorization.revoked", user.id).await;
    assert_eq!((events.len(), events[0].data["reason"].as_str()), (1, Some("user")));
    assert_eq!(ctx.security_events(user.id, "APP_ACCESS_REVOKED").await, 1);
}

#[tokio::test]
async fn a_deleted_or_disabled_application_stops_working_everywhere() {
    let ctx = start().await;
    let (dana, user) = (ctx.app.user("dana").await, ctx.app.user("user").await);
    let app = create_app(&ctx, &dana, &["identity:read"], CONFIDENTIAL).await;
    let (access, refresh_token) = obtain_tokens(&ctx, &user, &app, "identity:read").await;
    assert!(whoami(&ctx, &with_token(&user, &access)).await.is_ok());

    // Moderation takes it down (the report subject resolves while it is up).
    let subject = ctx
        .app
        .core
        .subjects
        .resolve(tokuly_proto::common::SubjectType::SUBJECT_TYPE_APP, &app.id, None)
        .await
        .unwrap();
    assert_eq!(
        (subject.owner_id, subject.title.as_str(), subject.path),
        (dana.id, "Test App", format!("/apps/{}", app.id))
    );
    let removed = serde_json::json!({ "subject_type": tokuly_proto::common::SubjectType::SUBJECT_TYPE_APP as i32, "subject_id": app.id, "reason": "phishing", "moderator_id": dana.id });
    ctx.dev.on_account_event(&event("admin.content.removed", &removed)).await.unwrap();
    // A takedown of something else is none of this domain's business.
    let other = serde_json::json!({ "subject_type": tokuly_proto::common::SubjectType::SUBJECT_TYPE_NOTE as i32, "subject_id": app.id, "reason": "x", "moderator_id": dana.id });
    ctx.dev.on_account_event(&event("admin.content.removed", &other)).await.unwrap();

    assert_eq!(whoami(&ctx, &with_token(&user, &access)).await.unwrap_err().reason, "TOKEN_REVOKED");
    assert_eq!(refresh(&ctx, &app, &refresh_token).await.error(), "invalid_client");
    let (_, challenge) = pkce();
    let refused = consent(&ctx, &user, authorization_request(&app, "identity:read", &challenge)).await.unwrap_err();
    // To users a disabled application is one that does not exist.
    assert_eq!((refused.code.as_str(), refused.reason.as_str()), ("not_found", "CLIENT_NOT_FOUND"));
    // Its owner cannot make the takedown disappear by deleting it.
    let delete = DeleteAppRequest { app_id: app.id.clone(), ..Default::default() };
    let kept = ctx
        .app
        .call::<_, DeleteAppResponse>(&format!("{APP_SERVICE}/DeleteApp"), Some(&dana), &delete)
        .await
        .unwrap_err();
    assert_eq!((kept.code.as_str(), kept.reason.as_str()), ("failed_precondition", "APP_DISABLED"));
    let public = ctx
        .app
        .call::<_, GetPublicAppResponse>(
            &format!("{AUTHORIZATION_SERVICE}/GetPublicApp"),
            None,
            &GetPublicAppRequest { app_id: app.id.clone(), ..Default::default() },
        )
        .await;
    assert_eq!(public.unwrap_err().reason, "APP_NOT_FOUND");
    assert!(
        ctx.app
            .core
            .subjects
            .resolve(tokuly_proto::common::SubjectType::SUBJECT_TYPE_APP, &app.id, None)
            .await
            .is_err()
    );
    // The owner sees why, and is told.
    let got: GetAppResponse = ctx
        .app
        .call(
            &format!("{APP_SERVICE}/GetApp"),
            Some(&dana),
            &GetAppRequest { app_id: app.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(
        (got.app.as_option().unwrap().status.as_known(), got.app.as_option().unwrap().disabled_reason.as_str()),
        (Some(AppStatus::APP_STATUS_DISABLED), "phishing")
    );
    assert_eq!(ctx.notifications(dana.id, "dev.app_disabled").await.len(), 1);

    // Deleting an application ends its tokens just the same.
    let second = create_app(&ctx, &dana, &["identity:read"], CONFIDENTIAL).await;
    let (access, _) = obtain_tokens(&ctx, &user, &second, "identity:read").await;
    assert!(whoami(&ctx, &with_token(&user, &access)).await.is_ok());
    ctx.app
        .call::<_, DeleteAppResponse>(
            &format!("{APP_SERVICE}/DeleteApp"),
            Some(&dana),
            &DeleteAppRequest { app_id: second.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(whoami(&ctx, &with_token(&user, &access)).await.unwrap_err().reason, "TOKEN_INVALID");
}

#[tokio::test]
async fn the_device_flow_lets_a_cli_in_once_its_user_agrees() {
    // No pause between polls, so the test need not wait.
    let ctx = start_with(DevConfig { device_poll_seconds: 0, ..config() }).await;
    let (dana, user) = (ctx.app.user("dana").await, ctx.app.user("user").await);
    let cli = create_cli(&ctx, &dana, &["identity:read", "notes:read"]).await;
    let started =
        ctx.form("/oauth/device_authorization", None, &[("client_id", &cli.id), ("scope", "identity:read")]).await;
    assert_eq!(started.status, 200, "{}", String::from_utf8_lossy(&started.body));
    let device = started.json();
    let (device_code, user_code) =
        (device["device_code"].as_str().unwrap().to_owned(), device["user_code"].as_str().unwrap().to_owned());
    assert_eq!(
        (device["verification_uri"].as_str(), device["expires_in"].as_i64(), device["interval"].as_i64()),
        (Some("http://localhost:8080/device"), Some(900), Some(0))
    );
    assert_eq!(device["verification_uri_complete"], format!("http://localhost:8080/device?user_code={user_code}"));
    assert!(user_code.len() == 9 && secrets::is(&device_code, secrets::SecretKind::Device));

    let poll = || {
        let (ctx, cli, device_code) = (&ctx, &cli, device_code.clone());
        async move {
            ctx.form(
                "/oauth/token",
                None,
                &[
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("device_code", &device_code),
                    ("client_id", &cli.id),
                ],
            )
            .await
        }
    };
    let pending = poll().await;
    assert_eq!((pending.status, pending.error().as_str()), (400, "authorization_pending"));

    // The device page: typed sloppily, shown as a consent screen.
    let typed = user_code.to_lowercase().replace('-', " ");
    let shown: GetDeviceConsentResponse = ctx
        .app
        .call(
            &format!("{AUTHORIZATION_SERVICE}/GetDeviceConsent"),
            Some(&user),
            &GetDeviceConsentRequest { user_code: typed.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let consent = shown.consent.as_option().unwrap();
    assert_eq!((consent.app.as_option().unwrap().id.as_str(), consent.scopes.len()), (cli.id.as_str(), 1));
    let wrong = GetDeviceConsentRequest { user_code: "BCDF-GHJK".into(), ..Default::default() };
    let wrong = ctx
        .app
        .call::<_, GetDeviceConsentResponse>(&format!("{AUTHORIZATION_SERVICE}/GetDeviceConsent"), Some(&user), &wrong)
        .await
        .unwrap_err();
    assert_eq!((wrong.code.as_str(), wrong.reason.as_str()), ("not_found", "DEVICE_CODE_INVALID"));
    assert_eq!(
        ctx.app
            .call::<_, ApproveDeviceResponse>(
                &format!("{AUTHORIZATION_SERVICE}/ApproveDevice"),
                None,
                &ApproveDeviceRequest { user_code: typed.clone(), ..Default::default() }
            )
            .await
            .unwrap_err()
            .reason,
        "SIGN_IN_REQUIRED"
    );

    ctx.app
        .call::<_, ApproveDeviceResponse>(
            &format!("{AUTHORIZATION_SERVICE}/ApproveDevice"),
            Some(&user),
            &ApproveDeviceRequest { user_code: typed.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    // The code is answered: it cannot be answered again.
    let again = ctx
        .app
        .call::<_, DenyDeviceResponse>(
            &format!("{AUTHORIZATION_SERVICE}/DenyDevice"),
            Some(&user),
            &DenyDeviceRequest { user_code: typed, ..Default::default() },
        )
        .await;
    assert_eq!(again.unwrap_err().reason, "DEVICE_CODE_INVALID");

    let granted = poll().await;
    assert_eq!(granted.status, 200, "{}", String::from_utf8_lossy(&granted.body));
    let tokens = granted.json();
    assert_eq!(tokens["scope"], "identity:read");
    let me = whoami(&ctx, &with_token(&user, tokens["access_token"].as_str().unwrap())).await.unwrap();
    assert_eq!(me.user.as_option().unwrap().id, user.id.to_string());
    // The device code is spent.
    assert_eq!(poll().await.error(), "expired_token");
    // And the CLI can refresh without a secret.
    let renewed = ctx
        .form(
            "/oauth/token",
            None,
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", tokens["refresh_token"].as_str().unwrap()),
                ("client_id", &cli.id),
            ],
        )
        .await;
    assert_eq!(renewed.status, 200);
}

#[tokio::test]
async fn the_device_flow_reports_denial_impatience_and_bad_requests() {
    let ctx = start().await;
    let (dana, user) = (ctx.app.user("dana").await, ctx.app.user("user").await);
    let cli = create_cli(&ctx, &dana, &["identity:read"]).await;
    let other = create_cli(&ctx, &dana, &["identity:read"]).await;
    // An application that did not ask for the device flow at registration does not get it.
    let plain = create_app(&ctx, &dana, &["identity:read"], PUBLIC).await;
    assert!(!plain.app.device_flow && cli.app.device_flow);
    let refused = ctx.form("/oauth/device_authorization", None, &[("client_id", &plain.id)]).await;
    assert_eq!((refused.status, refused.error().as_str()), (400, "unauthorized_client"));
    let start_device = |scope: &'static str| {
        let (ctx, cli) = (&ctx, &cli);
        async move { ctx.form("/oauth/device_authorization", None, &[("client_id", &cli.id), ("scope", scope)]).await }
    };
    let poll = |client: String, device_code: String| {
        let ctx = &ctx;
        async move {
            ctx.form(
                "/oauth/token",
                None,
                &[
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("device_code", &device_code),
                    ("client_id", &client),
                ],
            )
            .await
        }
    };
    assert_eq!(start_device("chat:read").await.error(), "invalid_scope");
    assert_eq!(
        ctx.form("/oauth/device_authorization", None, &[("client_id", "nobody")]).await.error(),
        "invalid_client"
    );
    assert_eq!(ctx.form("/oauth/device_authorization", None, &[]).await.error(), "invalid_client");

    // Polling faster than the interval (5 s by default) is told to slow down, and the interval grows.
    let device = start_device("").await.json();
    let device_code = device["device_code"].as_str().unwrap().to_owned();
    assert_eq!(device["interval"], 5);
    assert_eq!(poll(cli.id.clone(), device_code.clone()).await.error(), "authorization_pending");
    let hasty = poll(cli.id.clone(), device_code.clone()).await;
    assert_eq!((hasty.status, hasty.error().as_str()), (400, "slow_down"));
    assert!(hasty.json()["error_description"].as_str().unwrap().contains("10 seconds"));
    // Another client cannot poll it, and is told what it would be told about a code that never existed.
    let foreign = poll(other.id.clone(), device_code.clone()).await;
    let unknown = poll(other.id.clone(), secrets::generate(secrets::SecretKind::Device)).await;
    assert_eq!((foreign.status, foreign.json()), (unknown.status, unknown.json()));
    assert_eq!(foreign.error(), "expired_token");

    // The user says no.
    let deny = DenyDeviceRequest { user_code: device["user_code"].as_str().unwrap().to_owned(), ..Default::default() };
    ctx.app
        .call::<_, DenyDeviceResponse>(&format!("{AUTHORIZATION_SERVICE}/DenyDevice"), Some(&user), &deny)
        .await
        .unwrap();
    // The device still has to respect the interval before it learns of it;
    // the test lets that time pass by removing the marker.
    let key_hash = ctx.app.core.keys.token_hash(&device_code);
    ctx.app.core.cache.del(&format!("dev:oauth:device-wait:{key_hash}")).await.unwrap();
    let denied = poll(cli.id.clone(), device_code.clone()).await;
    assert_eq!((denied.status, denied.error().as_str()), (400, "access_denied"));
    assert_eq!(poll(cli.id.clone(), device_code).await.error(), "expired_token");
    // Codes that never existed.
    assert_eq!(poll(cli.id.clone(), secrets::generate(secrets::SecretKind::Device)).await.error(), "expired_token");
    assert_eq!(poll(cli.id.clone(), "garbage".to_owned()).await.error(), "expired_token");
    let listed: ListAuthorizedAppsResponse = ctx
        .app
        .call(
            &format!("{AUTHORIZATION_SERVICE}/ListAuthorizedApps"),
            Some(&user),
            &ListAuthorizedAppsRequest::default(),
        )
        .await
        .unwrap();
    assert!(listed.apps.is_empty(), "a denial grants nothing");
}

#[tokio::test]
async fn clients_configure_themselves_from_the_metadata_document() {
    let ctx = start().await;
    let reply = ctx.get("/.well-known/oauth-authorization-server").await;
    assert_eq!(reply.status, 200);
    let metadata = reply.json();
    assert_eq!(metadata["issuer"], "http://localhost:8080/api");
    assert_eq!(metadata["token_endpoint"], "http://localhost:8080/api/oauth/token");
    assert_eq!(metadata["authorization_endpoint"], "http://localhost:8080/oauth/authorize");
    assert_eq!(metadata["code_challenge_methods_supported"], serde_json::json!(["S256"]));
    assert_eq!(metadata["response_types_supported"], serde_json::json!(["code"]));
    assert!(
        metadata["grant_types_supported"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("urn:ietf:params:oauth:grant-type:device_code"))
    );
    assert!(metadata["scopes_supported"].as_array().unwrap().contains(&serde_json::json!("geo:read")));
    // The token endpoint only speaks forms, and only POST.
    let not_a_form = ctx.get("/oauth/token").await;
    assert_eq!(not_a_form.status, 405);
}
