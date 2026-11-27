//! What pushes show on the lock screen (「通知に内容を表示」): the choice,
//! its default by age, and that left-out text never enters the payload.
use tokuly_notify::{
    consumer::{PushDecision, RequestOutcome},
    events::{BIRTHDATE_GIVEN_TOPIC, USER_CREATED_TOPIC},
};
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

/// `identity.user.created` as sent before it carried `minor`.
async fn created(ctx: &Ctx, user: &TestUser) {
    let event = event(USER_CREATED_TOPIC, serde_json::json!({ "user_id": user.id, "handle": user.handle, "invite_code": "" }));
    ctx.notify.handle_event(event).await.unwrap();
}

/// `identity.user.created` with the age group of the sign-up day.
async fn created_as(ctx: &Ctx, user: &TestUser, minor: bool) {
    let payload = serde_json::json!({ "user_id": user.id, "handle": user.handle, "invite_code": "", "minor": minor });
    ctx.notify.handle_event(event(USER_CREATED_TOPIC, payload)).await.unwrap();
}

/// The account was made `years_ago`.
async fn signed_up(ctx: &Ctx, user: &TestUser, years_ago: i32) {
    sqlx::query("UPDATE users SET created_at = now() - make_interval(years => $2) WHERE id = $1")
        .bind(user.id)
        .bind(years_ago)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
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

/// The event can be handled late (retries, a redrive) — after an 18th
/// birthday, even. The age group is the one identity sent: the sign-up day's.
#[tokio::test]
async fn a_late_sign_up_event_keeps_the_age_of_the_sign_up_day() {
    let ctx = start().await;
    let (late, adult) = (ctx.app.user("late").await, ctx.app.user("adult").await);
    // Signed up two years ago, under 17; 18 by the time the event is handled.
    signed_up(&ctx, &late, 2).await;
    set_birthdate(&ctx, &late, 19).await;
    created_as(&ctx, &late, true).await;
    created_as(&ctx, &adult, false).await;
    assert_eq!(preview_of(&ctx, &late).await, PushPreview::PUSH_PREVIEW_SENDER_ONLY);
    assert_eq!(preview_of(&ctx, &adult).await, PushPreview::PUSH_PREVIEW_FULL);
}

/// Without the event (a first push before it, or one given up on) and for an
/// event from before it carried `minor`, the same rule: the age on the
/// sign-up day.
#[tokio::test]
async fn without_the_age_group_the_sign_up_day_decides() {
    let ctx = start_insecure().await;
    let (first_push, old_event) = (ctx.app.user("first").await, ctx.app.user("old").await);
    for user in [&first_push, &old_event] {
        // Signed up at 16 years and 11 months, two years ago.
        signed_up(&ctx, user, 2).await;
        set_birthdate(&ctx, user, 19).await;
    }
    created(&ctx, &old_event).await;
    assert_eq!(preview_of(&ctx, &first_push).await, PushPreview::PUSH_PREVIEW_SENDER_ONLY);
    assert_eq!(preview_of(&ctx, &old_event).await, PushPreview::PUSH_PREVIEW_SENDER_ONLY);
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

/// No birthdate counts as a minor (`tokuly_identity::age`), at sign-up as
/// everywhere else.
#[tokio::test]
async fn an_account_without_a_birthdate_starts_with_sender_only() {
    let ctx = start_insecure().await;
    let unknown = ctx.app.user("unknown").await;
    sqlx::query("UPDATE users SET birthdate = NULL WHERE id = $1").bind(unknown.id).execute(&ctx.app.core.db).await.unwrap();
    created(&ctx, &unknown).await;
    assert_eq!(preview_of(&ctx, &unknown).await, PushPreview::PUSH_PREVIEW_SENDER_ONLY);
}

async fn birthdate_given(ctx: &Ctx, user: &TestUser, minor: bool) {
    let event = event(BIRTHDATE_GIVEN_TOPIC, serde_json::json!({ "user_id": user.id, "minor": minor }));
    ctx.notify.handle_event(event).await.unwrap();
}

/// What an account from before the birthdate question has stored: full, by
/// the rule of its day.
async fn stored_full(ctx: &Ctx, user: &TestUser) {
    sqlx::query("INSERT INTO notify_settings (user_id, push_preview) VALUES ($1, 1)")
        .bind(user.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
}

/// A minor who gives the birthdate later gets the minors' default then,
/// unless they chose; it only narrows, and a redelivery changes nothing.
#[tokio::test]
async fn a_minor_giving_the_birthdate_later_gets_sender_only_unless_they_chose() {
    let ctx = start_insecure().await;
    let (teen, chose, hid, adult, fresh) = (
        ctx.app.user("teen").await,
        ctx.app.user("chose").await,
        ctx.app.user("hid").await,
        ctx.app.user("adult").await,
        ctx.app.user("fresh").await,
    );
    for user in [&teen, &adult] {
        stored_full(&ctx, user).await;
    }
    choose(&ctx, &chose, PushPreview::PUSH_PREVIEW_FULL).await;
    choose(&ctx, &hid, PushPreview::PUSH_PREVIEW_HIDDEN).await;

    for _ in 0..2 {
        for user in [&teen, &chose, &hid, &fresh] {
            birthdate_given(&ctx, user, true).await;
        }
        birthdate_given(&ctx, &adult, false).await;
    }
    assert_eq!(
        preview_of(&ctx, &teen).await,
        PushPreview::PUSH_PREVIEW_SENDER_ONLY,
        "a full stored before choices were told apart counts as not chosen",
    );
    assert_eq!(preview_of(&ctx, &fresh).await, PushPreview::PUSH_PREVIEW_SENDER_ONLY, "nothing stored yet");
    assert_eq!(preview_of(&ctx, &chose).await, PushPreview::PUSH_PREVIEW_FULL, "their own choice stays");
    assert_eq!(preview_of(&ctx, &hid).await, PushPreview::PUSH_PREVIEW_HIDDEN, "only narrows");
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

#[tokio::test]
async fn the_inbox_never_returns_an_unnamed_actor() {
    let ctx = start().await;
    let (creator, buyer) = (ctx.app.user("creator").await, ctx.app.user("buyer").await);
    let mut frames = ctx.app.listen_user(creator.id).await;
    // Stamp's notice when a pack sells: the buyer caused it but stays unnamed.
    let sale = tokuly_core::outbound::NotificationRequest::new(
        creator.id,
        "stamp.pack.sold",
        tokuly_core::outbound::Localized::new("スタンプが売れたよ!", "You sold a stamp pack!"),
        tokuly_core::outbound::Localized::new("「ねこ」が購入されたよ! +90コイン", "Someone bought “ねこ”. +90 coins"),
        "/stamps/studio",
    )
    .unnamed_actor(buyer.id)
    .collapse("stamp.pack.sold:ねこ");
    let id = ctx.deliver_to_inbox(&sale).await;
    let buyer_id = buyer.id.to_string();

    // ListNotifications
    let inbox = ctx.inbox(&creator).await;
    let entry = inbox.notifications.iter().find(|n| n.id == id).expect("the sale is in the inbox");
    assert!(entry.actor.as_option().is_none(), "no buyer in the list: {:?}", entry.actor);
    assert!(entry.actor_unnamed, "someone, unnamed");
    // The realtime push of the new entry.
    let frame = frames.next(std::time::Duration::from_secs(5)).await.expect("a realtime frame");
    let pushed = frame.event::<NotifyEvent>().expect("a notify frame");
    let Some(notify_event::Event::Received(received)) = pushed.event else { panic!("expected `received`") };
    assert!(received.notification.as_option().unwrap().actor.as_option().is_none(), "no buyer in the push");
    // The personal data export.
    let exporter = ctx.app.core.exporters.all().into_iter().find(|e| e.name() == "notify").unwrap();
    let export = exporter.export(creator.id).await.unwrap().to_string();
    assert!(!export.contains(&buyer_id) && !export.contains(&buyer.handle), "no buyer in the export: {export}");

    // A named sale folded into the same entry shows its actor again, and an
    // unnamed one after it hides it again.
    ctx.deliver_to_inbox(&tokuly_core::outbound::NotificationRequest { actor_unnamed: false, ..sale.clone() }).await;
    let shown = ctx.inbox(&creator).await.notifications.into_iter().find(|n| n.id == id).unwrap();
    assert_eq!(shown.actor.as_option().map(|a| a.id.as_str()), Some(buyer_id.as_str()));
    ctx.deliver_to_inbox(&sale).await;
    let hidden = ctx.inbox(&creator).await.notifications.into_iter().find(|n| n.id == id).unwrap();
    assert!(hidden.actor.as_option().is_none());

    // Blocking or muting the buyer changes nothing about such entries:
    // otherwise 「売れたよ」 vanishing (or the next one never arriving)
    // would tell the creator who bought.
    let block = event(USER_BLOCKED_TOPIC_NAME, serde_json::json!({ "user_id": creator.id, "blocked_id": buyer.id }));
    ctx.notify.handle_event(block).await.unwrap();
    assert!(ctx.inbox(&creator).await.notifications.iter().any(|n| n.id == id), "the sale stays after a block");
    let again = ctx.deliver(&sale).await;
    assert!(matches!(again, RequestOutcome::Handled { notification_id: Some(_), .. }), "a sale after the block still arrives: {again:?}");
    let other = ctx.app.user("other").await;
    let mute = tokuly_proto::notify::MuteUserRequest { user_id: buyer.id.to_string(), ..Default::default() };
    let _: tokuly_proto::notify::MuteUserResponse =
        ctx.app.call("tokuly.notify.v1.PreferencesService/MuteUser", Some(&other), &mute).await.unwrap();
    let to_other = tokuly_core::outbound::NotificationRequest { user_id: other.id, ..sale.clone() };
    let muted = ctx.deliver(&to_other).await;
    assert!(matches!(muted, RequestOutcome::Handled { notification_id: Some(_), .. }), "a mute does not hold it back: {muted:?}");
}

const USER_BLOCKED_TOPIC_NAME: &str = "social.user.blocked";
