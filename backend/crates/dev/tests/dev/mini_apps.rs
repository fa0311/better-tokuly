//! Mini apps: the owner's manifest, staff review and what people grant.
use tokuly_proto::dev::*;
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

use crate::common::*;

const MINI_APP_SERVICE: &str = "tokuly.dev.v1.MiniAppService";
const REVIEW_SERVICE: &str = "tokuly.dev.v1.MiniAppReviewService";
const LAUNCH_URL: &str = "https://warikan.example/app";

async fn call<Req, Res>(ctx: &Ctx, method: &str, caller: &TestUser, request: &Req) -> Result<Res, RpcError>
where
    Req: serde::Serialize,
    Res: serde::de::DeserializeOwned,
{
    let service =
        if matches!(method, "ListMiniAppReviews" | "ReviewMiniApp") { REVIEW_SERVICE } else { MINI_APP_SERVICE };
    ctx.app.call(&format!("{service}/{method}"), Some(caller), request).await
}

fn manifest(app_id: &str, launch_url: &str, scopes: &[&str]) -> SetMiniAppManifestRequest {
    SetMiniAppManifestRequest {
        app_id: app_id.into(),
        name: "割り勘".into(),
        launch_url: launch_url.into(),
        scopes: scopes.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

async fn set(ctx: &Ctx, owner: &TestUser, request: &SetMiniAppManifestRequest) -> MiniAppReview {
    let set: SetMiniAppManifestResponse =
        call(ctx, "SetMiniAppManifest", owner, request).await.expect("manifest is set");
    set.review.into_option().expect("review")
}

async fn submit(ctx: &Ctx, owner: &TestUser, app_id: &str) -> Result<MiniAppReview, RpcError> {
    let request = SubmitMiniAppRequest { app_id: app_id.into(), ..Default::default() };
    call::<_, SubmitMiniAppResponse>(ctx, "SubmitMiniApp", owner, &request)
        .await
        .map(|submitted| submitted.review.into_option().expect("review"))
}

async fn review(
    ctx: &Ctx,
    moderator: &TestUser,
    app_id: &str,
    revision: i32,
    approve: bool,
    note: &str,
) -> Result<MiniAppReview, RpcError> {
    let request =
        ReviewMiniAppRequest { app_id: app_id.into(), revision, approve, note: note.into(), ..Default::default() };
    call::<_, ReviewMiniAppResponse>(ctx, "ReviewMiniApp", moderator, &request)
        .await
        .map(|reviewed| reviewed.review.into_option().expect("review"))
}

async fn open(ctx: &Ctx, person: &TestUser, app_id: &str) -> Result<GetMiniAppResponse, RpcError> {
    call(ctx, "GetMiniApp", person, &GetMiniAppRequest { app_id: app_id.into(), ..Default::default() }).await
}

async fn grant(ctx: &Ctx, person: &TestUser, app_id: &str, scopes: &[&str]) -> Result<GrantMiniAppResponse, RpcError> {
    let request = GrantMiniAppRequest {
        app_id: app_id.into(),
        scopes: scopes.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    call(ctx, "GrantMiniApp", person, &request).await
}

/// An application of `owner` with an approved mini app asking for `scopes`.
async fn approved(ctx: &Ctx, owner: &TestUser, moderator: &TestUser, scopes: &[&str]) -> String {
    let app = create_app(ctx, owner, &["notes:read"], AppClientType::APP_CLIENT_TYPE_PUBLIC).await;
    let draft = set(ctx, owner, &manifest(&app.id, LAUNCH_URL, scopes)).await;
    submit(ctx, owner, &app.id).await.expect("submitted");
    review(ctx, moderator, &app.id, draft.revision, true, "").await.expect("approved");
    app.id
}

#[tokio::test]
async fn people_open_only_the_approved_copy() {
    let ctx = start().await;
    let (dana, mod_, eve) =
        (ctx.app.user("dana").await, ctx.app.user_with_roles("mod", &["moderator"]).await, ctx.app.user("eve").await);
    let app = create_app(&ctx, &dana, &["notes:read"], AppClientType::APP_CLIENT_TYPE_PUBLIC).await;
    let icon = ctx.images.upload(dana.id);
    let draft = set(
        &ctx,
        &dana,
        &SetMiniAppManifestRequest {
            icon_asset_id: Some(icon.to_string()),
            ..manifest(&app.id, LAUNCH_URL, &["friends", "profile"])
        },
    )
    .await;
    assert_eq!(draft.state.as_known(), Some(MiniAppReviewState::MINI_APP_REVIEW_STATE_DRAFT));
    assert_eq!(draft.draft.as_option().unwrap().scopes, ["profile", "friends"], "catalogue order");
    assert!(draft.live.as_option().is_none());
    assert_eq!(
        open(&ctx, &eve, &app.id).await.unwrap_err().reason,
        "MINI_APP_NOT_FOUND",
        "a draft is nobody's to open"
    );

    let waiting = submit(&ctx, &dana, &app.id).await.unwrap();
    assert_eq!(waiting.state.as_known(), Some(MiniAppReviewState::MINI_APP_REVIEW_STATE_IN_REVIEW));
    // Submitting again while it waits changes nothing.
    assert_eq!(submit(&ctx, &dana, &app.id).await.unwrap().submitted_at, waiting.submitted_at);
    assert_eq!(open(&ctx, &eve, &app.id).await.unwrap_err().reason, "MINI_APP_NOT_FOUND", "nor is one in review");
    let queue: ListMiniAppReviewsResponse =
        call(&ctx, "ListMiniAppReviews", &mod_, &ListMiniAppReviewsRequest::default()).await.unwrap();
    let queued = queue.reviews.iter().find(|r| r.app_id == app.id).expect("in the queue");
    assert_eq!(queued.app.as_option().unwrap().owner.as_option().unwrap().handle, dana.handle);

    let decided = review(&ctx, &mod_, &app.id, queued.revision, true, "").await.unwrap();
    assert_eq!(decided.state.as_known(), Some(MiniAppReviewState::MINI_APP_REVIEW_STATE_APPROVED));
    assert_eq!(ctx.notifications(dana.id, "dev.mini_app_reviewed").await.len(), 1);
    let opened = open(&ctx, &eve, &app.id).await.unwrap();
    let mini_app = opened.mini_app.as_option().unwrap();
    assert_eq!(
        (mini_app.name.as_str(), mini_app.launch_url.as_str(), mini_app.origin.as_str()),
        ("割り勘", LAUNCH_URL, "https://warikan.example")
    );
    assert_eq!(mini_app.scopes.iter().map(|s| s.scope.as_str()).collect::<Vec<_>>(), ["profile", "friends"]);
    assert_eq!(mini_app.developer.as_option().unwrap().id, dana.id.to_string());
    assert!(opened.granted_scopes.is_empty());
    assert_eq!(ctx.images.held_by(&format!("dev:mini-app-live-icon:{}", app.id)), Some(icon));

    // An edit is a new draft; people keep the approved copy until it is approved too.
    let other_icon = ctx.images.upload(dana.id);
    let edit = SetMiniAppManifestRequest {
        icon_asset_id: Some(other_icon.to_string()),
        ..manifest(&app.id, "https://elsewhere.example/", &["profile"])
    };
    let edited = set(&ctx, &dana, &edit).await;
    assert_eq!(
        (edited.state.as_known(), edited.revision),
        (Some(MiniAppReviewState::MINI_APP_REVIEW_STATE_DRAFT), decided.revision + 1)
    );
    assert_eq!(edited.live.as_option().unwrap().launch_url, LAUNCH_URL);
    assert_eq!(open(&ctx, &eve, &app.id).await.unwrap().mini_app.as_option().unwrap().launch_url, LAUNCH_URL);
    assert_eq!(ctx.images.held_by(&format!("dev:mini-app-live-icon:{}", app.id)), Some(icon), "the live icon stays");
    assert_eq!(ctx.images.held_by(&format!("dev:mini-app-icon:{}", app.id)), Some(other_icon));

    // Moderation takes the application down: the mini app is gone with it.
    ctx.dev.disable_app(Uuid::parse_str(&app.id).unwrap(), "spam").await.unwrap();
    assert_eq!(open(&ctx, &eve, &app.id).await.unwrap_err().reason, "MINI_APP_NOT_FOUND");
    let request = DeleteMiniAppRequest { app_id: app.id.clone(), ..Default::default() };
    let kept = call::<_, DeleteMiniAppResponse>(&ctx, "DeleteMiniApp", &dana, &request).await.unwrap_err();
    assert_eq!(kept.reason, "APP_DISABLED", "what moderation took down stays on record");
}

#[tokio::test]
async fn manifests_are_validated() {
    let ctx = start().await;
    let (dana, eve) = (ctx.app.user("dana").await, ctx.app.user("eve").await);
    let app = create_app(&ctx, &dana, &["notes:read"], AppClientType::APP_CLIENT_TYPE_PUBLIC).await;
    for (request, reason) in [
        (SetMiniAppManifestRequest { name: " ".into(), ..manifest(&app.id, LAUNCH_URL, &[]) }, "MINI_APP_NAME_INVALID"),
        (
            SetMiniAppManifestRequest { name: "あ".repeat(31), ..manifest(&app.id, LAUNCH_URL, &[]) },
            "MINI_APP_NAME_INVALID",
        ),
        (manifest(&app.id, "http://warikan.example/", &[]), "LAUNCH_URL_INVALID"),
        (manifest(&app.id, "http://127.0.0.1:5180/", &[]), "LAUNCH_URL_INVALID"),
        (manifest(&app.id, "https://warikan.example/#x", &[]), "LAUNCH_URL_INVALID"),
        (manifest(&app.id, LAUNCH_URL, &["geo:read"]), "MINI_APP_SCOPE_UNKNOWN"),
        (manifest(&app.id, LAUNCH_URL, &["location"]), "MINI_APP_SCOPE_UNKNOWN"),
    ] {
        let err = call::<_, SetMiniAppManifestResponse>(&ctx, "SetMiniAppManifest", &dana, &request).await.unwrap_err();
        assert_eq!(err.reason, reason, "{request:?}");
    }
    let theirs =
        call::<_, SetMiniAppManifestResponse>(&ctx, "SetMiniAppManifest", &eve, &manifest(&app.id, LAUNCH_URL, &[]))
            .await;
    assert_eq!(theirs.unwrap_err().reason, "APP_NOT_FOUND", "only the owner writes a manifest");
    assert_eq!(submit(&ctx, &dana, &app.id).await.unwrap_err().reason, "MINI_APP_NOT_FOUND");
    let request = GetMiniAppReviewRequest { app_id: app.id.clone(), ..Default::default() };
    assert_eq!(
        call::<_, GetMiniAppReviewResponse>(&ctx, "GetMiniAppReview", &dana, &request).await.unwrap_err().reason,
        "MINI_APP_NOT_FOUND"
    );

    // Development servers may allow a loopback page; production never does.
    let loopback = start_with(tokuly_dev::DevConfig { mini_app_allow_loopback: true, ..config() }).await;
    let owner = loopback.app.user("dana").await;
    let local = create_app(&loopback, &owner, &["notes:read"], AppClientType::APP_CLIENT_TYPE_PUBLIC).await;
    assert_eq!(
        set(&loopback, &owner, &manifest(&local.id, "http://127.0.0.1:5180/", &[]))
            .await
            .draft
            .as_option()
            .unwrap()
            .launch_url,
        "http://127.0.0.1:5180/"
    );
}

#[tokio::test]
async fn a_review_names_its_revision_and_never_its_owner() {
    let ctx = start().await;
    let dana = ctx.app.user_with_roles("dana", &["moderator"]).await;
    let (mod_a, mod_b, eve) = (
        ctx.app.user_with_roles("moda", &["moderator"]).await,
        ctx.app.user_with_roles("modb", &["moderator"]).await,
        ctx.app.user("eve").await,
    );
    let app = create_app(&ctx, &dana, &["notes:read"], AppClientType::APP_CLIENT_TYPE_PUBLIC).await;
    let first = set(&ctx, &dana, &manifest(&app.id, LAUNCH_URL, &[])).await;
    submit(&ctx, &dana, &app.id).await.unwrap();

    assert_eq!(review(&ctx, &eve, &app.id, first.revision, true, "").await.unwrap_err().reason, "FORBIDDEN");
    let list =
        call::<_, ListMiniAppReviewsResponse>(&ctx, "ListMiniAppReviews", &eve, &ListMiniAppReviewsRequest::default())
            .await;
    assert_eq!(list.unwrap_err().reason, "FORBIDDEN");
    assert_eq!(review(&ctx, &dana, &app.id, first.revision, true, "").await.unwrap_err().reason, "OWN_APP");
    assert_eq!(
        review(&ctx, &mod_a, &app.id, first.revision, false, "  ").await.unwrap_err().reason,
        "REVIEW_NOTE_REQUIRED"
    );
    assert_eq!(
        review(&ctx, &mod_a, &app.id, first.revision, false, &"x".repeat(501)).await.unwrap_err().reason,
        "REVIEW_NOTE_TOO_LONG"
    );

    // The owner changed it after the moderator read it: that decision is about something else.
    let second = set(&ctx, &dana, &manifest(&app.id, "https://other.example/", &[])).await;
    submit(&ctx, &dana, &app.id).await.unwrap();
    assert_eq!(review(&ctx, &mod_a, &app.id, first.revision, true, "").await.unwrap_err().reason, "REVIEW_OUTDATED");

    // Two moderators decide at once: one decision stands.
    let (approve, reject) = tokio::join!(
        review(&ctx, &mod_a, &app.id, second.revision, true, ""),
        review(&ctx, &mod_b, &app.id, second.revision, false, "広告ばかり"),
    );
    assert_eq!([approve.is_ok(), reject.is_ok()].iter().filter(|done| **done).count(), 1, "{approve:?} {reject:?}");
    let loser = approve.err().or(reject.err()).unwrap();
    assert_eq!(loser.reason, "REVIEW_OUTDATED");

    // A decided draft goes back only after a change.
    assert_eq!(submit(&ctx, &dana, &app.id).await.unwrap_err().reason, "MINI_APP_UNCHANGED");
}

#[tokio::test]
async fn an_edit_and_a_review_at_once_never_approve_the_unread_edit() {
    let ctx = start().await;
    let (dana, mod_) = (ctx.app.user("dana").await, ctx.app.user_with_roles("mod", &["moderator"]).await);
    let app = create_app(&ctx, &dana, &["notes:read"], AppClientType::APP_CLIENT_TYPE_PUBLIC).await;
    let read = set(&ctx, &dana, &manifest(&app.id, LAUNCH_URL, &[])).await;
    submit(&ctx, &dana, &app.id).await.unwrap();
    let unread = manifest(&app.id, "https://evil.example/", &["profile", "friends"]);
    let (edit, decision) = tokio::join!(
        call::<_, SetMiniAppManifestResponse>(&ctx, "SetMiniAppManifest", &dana, &unread),
        review(&ctx, &mod_, &app.id, read.revision, true, ""),
    );
    edit.unwrap();
    let request = GetMiniAppReviewRequest { app_id: app.id.clone(), ..Default::default() };
    let now: GetMiniAppReviewResponse = call(&ctx, "GetMiniAppReview", &dana, &request).await.unwrap();
    let now = now.review.into_option().unwrap();
    match decision {
        // Approved first: the live copy is what the moderator read; the edit is a fresh draft.
        Ok(_) => assert_eq!(now.live.as_option().unwrap().launch_url, LAUNCH_URL),
        Err(err) => {
            assert_eq!(err.reason, "REVIEW_OUTDATED");
            assert!(now.live.as_option().is_none());
        }
    }
    assert_eq!(now.state.as_known(), Some(MiniAppReviewState::MINI_APP_REVIEW_STATE_DRAFT));
}

#[tokio::test]
async fn people_grant_only_what_the_approved_copy_asks_for() {
    let ctx = start().await;
    let (dana, mod_, eve) =
        (ctx.app.user("dana").await, ctx.app.user_with_roles("mod", &["moderator"]).await, ctx.app.user("eve").await);
    let app_id = approved(&ctx, &dana, &mod_, &["profile"]).await;
    assert_eq!(grant(&ctx, &eve, &app_id, &["friends"]).await.unwrap_err().reason, "MINI_APP_SCOPE_UNKNOWN");
    assert_eq!(grant(&ctx, &eve, &app_id, &["profile"]).await.unwrap().granted_scopes, ["profile"]);
    assert_eq!(open(&ctx, &eve, &app_id).await.unwrap().granted_scopes, ["profile"]);
    // The grant is the person's: somebody else has granted nothing.
    assert!(open(&ctx, &dana, &app_id).await.unwrap().granted_scopes.is_empty());

    // A later approved copy that asks for less narrows what counts.
    set(&ctx, &dana, &manifest(&app_id, LAUNCH_URL, &["friends"])).await;
    let draft = submit(&ctx, &dana, &app_id).await.unwrap();
    review(&ctx, &mod_, &app_id, draft.revision, true, "").await.unwrap();
    assert!(open(&ctx, &eve, &app_id).await.unwrap().granted_scopes.is_empty(), "profile is no longer asked for");
    assert_eq!(grant(&ctx, &eve, &app_id, &["friends"]).await.unwrap().granted_scopes, ["friends"]);

    let request = RevokeMiniAppGrantRequest { app_id: app_id.clone(), ..Default::default() };
    call::<_, RevokeMiniAppGrantResponse>(&ctx, "RevokeMiniAppGrant", &eve, &request).await.unwrap();
    assert!(open(&ctx, &eve, &app_id).await.unwrap().granted_scopes.is_empty());

    // Unknown and malformed mini apps read the same.
    assert_eq!(open(&ctx, &eve, &Uuid::now_v7().to_string()).await.unwrap_err().reason, "MINI_APP_NOT_FOUND");
    assert_eq!(open(&ctx, &eve, "nope").await.unwrap_err().reason, "MINI_APP_NOT_FOUND");

    // A token is not a person consenting.
    let (_, secret) = personal_token(&ctx, &eve, &["identity:read"]).await;
    let token = with_token(&eve, &secret);
    assert_eq!(open(&ctx, &token, &app_id).await.unwrap_err().reason, "SESSION_REQUIRED");
    assert_eq!(grant(&ctx, &token, &app_id, &["friends"]).await.unwrap_err().reason, "SESSION_REQUIRED");
}

#[tokio::test]
async fn deleting_takes_the_mini_app_and_its_grants() {
    let ctx = start().await;
    let (dana, mod_, eve) =
        (ctx.app.user("dana").await, ctx.app.user_with_roles("mod", &["moderator"]).await, ctx.app.user("eve").await);
    let app_id = approved(&ctx, &dana, &mod_, &["profile"]).await;
    grant(&ctx, &eve, &app_id, &["profile"]).await.unwrap();
    let request = DeleteMiniAppRequest { app_id: app_id.clone(), ..Default::default() };
    call::<_, DeleteMiniAppResponse>(&ctx, "DeleteMiniApp", &dana, &request).await.unwrap();
    assert_eq!(open(&ctx, &eve, &app_id).await.unwrap_err().reason, "MINI_APP_NOT_FOUND");
    let grants: i64 = sqlx::query_scalar("SELECT count(*) FROM dev_mini_app_grants WHERE app_id = $1")
        .bind(Uuid::parse_str(&app_id).unwrap())
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(grants, 0);
    assert_eq!(
        call::<_, DeleteMiniAppResponse>(&ctx, "DeleteMiniApp", &dana, &request).await.unwrap_err().reason,
        "MINI_APP_NOT_FOUND"
    );

    // A deleted account's grants go with it.
    let other = approved(&ctx, &dana, &mod_, &["profile"]).await;
    grant(&ctx, &eve, &other, &["profile"]).await.unwrap();
    ctx.dev.on_user_deleted(eve.id).await.unwrap();
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM dev_mini_app_grants WHERE user_id = $1")
        .bind(eve.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(left, 0);
}
