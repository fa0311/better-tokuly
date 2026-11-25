//! What pushes show on the lock screen (「通知に内容を表示」): the choice,
//! its default by age, and that left-out text never enters the payload.
use tokuly_notify::{consumer::PushDecision, consumer::RequestOutcome, events::USER_CREATED_TOPIC};
use tokuly_proto::notify::*;
use tokuly_testkit::TestUser;

use crate::common::*;

async fn preview_of(ctx: &Ctx, user: &TestUser) -> PushPreview {
    let response: GetPreferencesResponse = ctx
        .app
        .call("tokuly.notify.v1.PreferencesService/GetPreferences", Some(user), &GetPreferencesRequest::default())
        .await
        .unwrap();
    response.preferences.as_option().unwrap().push_preview.as_known().unwrap()
}

async fn choose(ctx: &Ctx, user: &TestUser, preview: PushPreview) {
    update_preferences(ctx, user, UpdatePreferencesRequest { push_preview: preview.into(), ..Default::default() }).await;
}

async fn created(ctx: &Ctx, user: &TestUser) {
    let event = event(USER_CREATED_TOPIC, serde_json::json!({ "user_id": user.id, "handle": user.handle, "invite_code": "" }));
    ctx.notify.handle_event(event).await.unwrap();
}

async fn set_birthdate(ctx: &Ctx, user: &TestUser, years_ago: i32) {
    sqlx::query("UPDATE users SET birthdate = (current_date - make_interval(years => $2))::date + 30 WHERE id = $1")
        .bind(user.id)
        .bind(years_ago)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
}

/// Delivers a chat message from `from` to `to`'s browser and returns the
/// decrypted push and the raw encrypted body.
async fn pushed(ctx: &Ctx, to: &TestUser, from: &TestUser, stub: &Stub, path: &str) -> (serde_json::Value, Vec<u8>) {
    pushed_keyed(ctx, to, from, stub, path, None).await
}

/// [`pushed`] with a collapse key on the request.
async fn pushed_keyed(
    ctx: &Ctx,
    to: &TestUser,
    from: &TestUser,
    stub: &Stub,
    path: &str,
    collapse_key: Option<&str>,
) -> (serde_json::Value, Vec<u8>) {
    let subscription = Subscription::new(&format!("{}{path}", stub.url));
    ctx.register(to, &RegisterDeviceRequest { locale: "ja".into(), ..subscription.request() }).await;
    let request = chat_message(to, from)
        .image("https://cdn.example/secret-photo.webp")
        .data(serde_json::json!({ "conversation_id": "42", "excerpt": "ひみつの話" }));
    let request = match collapse_key {
        Some(key) => request.collapse(key),
        None => request,
    };
    let job = ctx.deliver_for_push(&request).await;
    assert_eq!(ctx.notify.deliver_push(&job).await.unwrap().delivered, 1);
    let sent = stub.requests_to(path).pop().expect("a push was sent");
    (subscription.decrypt(&sent.body), sent.body)
}

#[tokio::test]
async fn each_choice_decides_what_the_push_payload_carries() {
    let ctx = start_insecure().await;
    let stub = Stub::accepting().await;
    let aoi = ctx.app.user("あおい").await;

    // すべて表示
    let full = ctx.app.user("full").await;
    assert_eq!(preview_of(&ctx, &full).await, PushPreview::PUSH_PREVIEW_FULL);
    let (message, _) = pushed(&ctx, &full, &aoi, &stub, "/push/full").await;
    assert_eq!(message["notification"]["title"], "新着メッセージ");
    assert_eq!(message["notification"]["body"], "やっほー！");
    assert_eq!(message["notification"]["image"], "https://cdn.example/secret-photo.webp");

    // 送り主だけ
    let sender = ctx.app.user("sender").await;
    choose(&ctx, &sender, PushPreview::PUSH_PREVIEW_SENDER_ONLY).await;
    assert_eq!(preview_of(&ctx, &sender).await, PushPreview::PUSH_PREVIEW_SENDER_ONLY);
    let (message, body) = pushed(&ctx, &sender, &aoi, &stub, "/push/sender").await;
    let notification = &message["notification"];
    assert_eq!(notification["title"], "チャット・通話");
    assert_eq!(notification["body"], "あおいさんからメッセージが届いたよ");
    assert_eq!(notification["data"]["path"], "/chat/42", "a tap still opens the chat");
    let decrypted = message.to_string();
    for hidden in ["やっほー", "新着メッセージ", "secret-photo", "ひみつの話", "conversation_id"] {
        assert!(!decrypted.contains(hidden), "{hidden:?} must not be in the payload: {decrypted}");
    }
    assert!(!String::from_utf8_lossy(&body).contains("やっほー"));

    // 表示しない
    let hidden = ctx.app.user("hidden").await;
    choose(&ctx, &hidden, PushPreview::PUSH_PREVIEW_HIDDEN).await;
    let (message, _) = pushed_keyed(&ctx, &hidden, &aoi, &stub, "/push/hidden", Some(&format!("social.buzz:{}", aoi.id))).await;
    let notification = &message["notification"];
    assert_eq!((notification["title"].as_str(), notification["body"].as_str()), (Some("Tokuly"), Some("新しいお知らせがあるよ")));
    assert_eq!(notification["navigate"], "http://localhost:8080/notifications", "a tap opens the inbox");
    let decrypted = message.to_string();
    assert!(!decrypted.contains(&aoi.id.to_string()), "the peer's ID from the collapse key: {decrypted}");
    for hidden_text in ["やっほー", "新着メッセージ", "secret-photo", "ひみつの話", "あおい", aoi.handle.as_str(), "/chat/42", "chat"] {
        assert!(!decrypted.contains(hidden_text), "{hidden_text:?} must not be in the payload: {decrypted}");
    }
    // No feature, no IDs: a generic kind and tag, nothing pointing at the entry.
    assert_eq!(notification["data"]["kind"], "notification");
    assert_eq!(notification["data"]["category"], "general");
    assert_eq!(notification["tag"], "notification");
    assert!(notification["data"].get("id").is_none_or(|id| id.is_null()), "{decrypted}");

    // The inbox keeps everything.
    let inbox = ctx.inbox(&hidden).await;
    assert_eq!(inbox.notifications[0].title, "新着メッセージ");
    assert_eq!(inbox.notifications[0].body, "やっほー！");
}

#[tokio::test]
async fn a_change_applies_to_pushes_still_waiting() {
    let ctx = start_insecure().await;
    let stub = Stub::accepting().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let subscription = Subscription::new(&format!("{}/push/waiting", stub.url));
    ctx.register(&alice, &RegisterDeviceRequest { locale: "ja".into(), ..subscription.request() }).await;
    let RequestOutcome::Handled { push: PushDecision::Queued(job), .. } = ctx.deliver(&chat_message(&alice, &bob)).await else {
        panic!("a push was queued");
    };
    choose(&ctx, &alice, PushPreview::PUSH_PREVIEW_HIDDEN).await;
    ctx.notify.deliver_push(&job).await.unwrap();
    let message = subscription.decrypt(&stub.requests_to("/push/waiting")[0].body);
    assert_eq!(message["notification"]["body"], "新しいお知らせがあるよ");
}

#[tokio::test]
async fn the_default_is_decided_by_age_when_the_account_is_created() {
    let ctx = start().await;
    let (adult, minor) = (ctx.app.user("adult").await, ctx.app.user("minor").await);
    set_birthdate(&ctx, &minor, 15).await;
    created(&ctx, &adult).await;
    created(&ctx, &minor).await;
    assert_eq!(preview_of(&ctx, &adult).await, PushPreview::PUSH_PREVIEW_FULL);
    assert_eq!(preview_of(&ctx, &minor).await, PushPreview::PUSH_PREVIEW_SENDER_ONLY);

    // Stored, not recomputed: turning 18 changes nothing, nor does the
    // event arriving again.
    set_birthdate(&ctx, &minor, 20).await;
    created(&ctx, &minor).await;
    assert_eq!(preview_of(&ctx, &minor).await, PushPreview::PUSH_PREVIEW_SENDER_ONLY);

    // The owner's choice wins, even one made before the event was handled.
    choose(&ctx, &minor, PushPreview::PUSH_PREVIEW_FULL).await;
    assert_eq!(preview_of(&ctx, &minor).await, PushPreview::PUSH_PREVIEW_FULL);
    let early = ctx.app.user("early").await;
    set_birthdate(&ctx, &early, 14).await;
    choose(&ctx, &early, PushPreview::PUSH_PREVIEW_HIDDEN).await;
    created(&ctx, &early).await;
    assert_eq!(preview_of(&ctx, &early).await, PushPreview::PUSH_PREVIEW_HIDDEN);
    // Changing quiet hours first does not count as a choice.
    let quiet = ctx.app.user("quiet").await;
    set_birthdate(&ctx, &quiet, 16).await;
    let hours = QuietHours { enabled: true, start_minute: 1320, end_minute: 420, ..Default::default() };
    update_preferences(&ctx, &quiet, UpdatePreferencesRequest { quiet_hours: hours.into(), ..Default::default() }).await;
    created(&ctx, &quiet).await;
    assert_eq!(preview_of(&ctx, &quiet).await, PushPreview::PUSH_PREVIEW_SENDER_ONLY);
}

#[tokio::test]
async fn unknown_choices_are_refused_and_unspecified_keeps_the_choice() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    choose(&ctx, &alice, PushPreview::PUSH_PREVIEW_HIDDEN).await;
    update_preferences(&ctx, &alice, UpdatePreferencesRequest::default()).await;
    assert_eq!(preview_of(&ctx, &alice).await, PushPreview::PUSH_PREVIEW_HIDDEN);
    let request = serde_json::json!({ "pushPreview": 9 });
    let err = ctx
        .app
        .call::<_, UpdatePreferencesResponse>("tokuly.notify.v1.PreferencesService/UpdatePreferences", Some(&alice), &request)
        .await
        .unwrap_err();
    assert_eq!(err.reason, "PUSH_PREVIEW_INVALID");
}

#[tokio::test]
async fn a_minor_whose_sign_up_event_never_arrived_still_gets_sender_only() {
    let ctx = start_insecure().await;
    let stub = Stub::accepting().await;
    let aoi = ctx.app.user("あおい").await;
    // identity.user.created was never handled (dropped after its retries).
    let minor = ctx.app.user("minor").await;
    set_birthdate(&ctx, &minor, 15).await;
    let (message, _) = pushed(&ctx, &minor, &aoi, &stub, "/push/minor-first").await;
    assert_eq!(message["notification"]["body"], "あおいさんからメッセージが届いたよ");
    assert!(!message.to_string().contains("やっほー"));
    // Decided once and stored: the 18th birthday changes nothing.
    set_birthdate(&ctx, &minor, 20).await;
    assert_eq!(preview_of(&ctx, &minor).await, PushPreview::PUSH_PREVIEW_SENDER_ONLY);

    // An adult in the same situation sees everything, and it is stored too.
    let adult = ctx.app.user("adult").await;
    assert_eq!(preview_of(&ctx, &adult).await, PushPreview::PUSH_PREVIEW_FULL);
    set_birthdate(&ctx, &adult, 15).await;
    assert_eq!(preview_of(&ctx, &adult).await, PushPreview::PUSH_PREVIEW_FULL);
}

#[tokio::test]
async fn sender_only_never_names_an_actor_the_full_text_leaves_out() {
    let ctx = start_insecure().await;
    let stub = Stub::accepting().await;
    let (creator, buyer) = (ctx.app.user("creator").await, ctx.app.user("buyer").await);
    choose(&ctx, &creator, PushPreview::PUSH_PREVIEW_SENDER_ONLY).await;
    let subscription = Subscription::new(&format!("{}/push/creator", stub.url));
    ctx.register(&creator, &RegisterDeviceRequest { locale: "ja".into(), ..subscription.request() }).await;
    // What stamp sends when a pack sells: the buyer is the actor (for mutes
    // and blocks) but the text says "someone".
    let request = tokuly_core::outbound::NotificationRequest::new(
        creator.id,
        "stamp.pack.sold",
        tokuly_core::outbound::Localized::new("スタンプが売れたよ!", "You sold a stamp pack!"),
        tokuly_core::outbound::Localized::new("「ねこ」が購入されたよ! +90コイン", "Someone bought “ねこ”. +90 coins"),
        "/stamps/studio",
    )
    .unnamed_actor(buyer.id);
    let job = ctx.deliver_for_push(&request).await;
    ctx.notify.deliver_push(&job).await.unwrap();
    let message = subscription.decrypt(&stub.requests_to("/push/creator")[0].body);
    let decrypted = message.to_string();
    assert!(!decrypted.contains("buyer"), "the buyer is named: {decrypted}");
    assert_eq!(message["notification"]["body"], "スタンプ・絵文字のお知らせが届いたよ");
}
