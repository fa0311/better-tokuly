//! Migration 1202: accounts that signed up before the birthdate question keep
//! the minors' preview their sign-up gave them. A test binary of its own: the
//! migration touches every account of the database.
use tokuly_notify::{events::BIRTHDATE_GIVEN_TOPIC, preferences::push_preview_of};
use tokuly_proto::notify::PushPreview;
use tokuly_testkit::TestUser;

#[allow(dead_code)]
#[path = "notify/common.rs"]
mod common;
use common::*;

async fn sql(ctx: &Ctx, query: &'static str, user: &TestUser) {
    sqlx::query(query).bind(user.id).execute(&ctx.app.core.db).await.unwrap();
}

/// Signed up two years ago, before the question: no birthdate, nothing stored.
async fn signed_up_without_birthdate(ctx: &Ctx, user: &TestUser) {
    sql(ctx, "UPDATE users SET birthdate = NULL, created_at = now() - interval '2 years' WHERE id = $1", user).await;
}

/// `SetBirthdate` with an adult's date, and the event it sends.
async fn gives_adult_birthdate(ctx: &Ctx, user: &TestUser) {
    sql(ctx, "UPDATE users SET birthdate = current_date - interval '30 years' WHERE id = $1", user).await;
    let given = event(BIRTHDATE_GIVEN_TOPIC, serde_json::json!({ "user_id": user.id, "minor": false }));
    ctx.notify.handle_event(given).await.unwrap();
}

async fn preview(ctx: &Ctx, user: &TestUser) -> PushPreview {
    push_preview_of(&ctx.app.core, user.id).await.unwrap()
}

/// No birthdate at sign-up counts as a minor. Once one is given, the first
/// push could not tell it was not there at sign-up (`identity.user.created`
/// lost, or from before it carried `minor`): the migration stores the
/// minors' preview while the absence is still visible.
#[tokio::test]
async fn accounts_without_a_birthdate_keep_the_minors_preview() {
    let ctx = start().await;
    let (legacy, defaulted, chose, hid, adult, bot) = (
        ctx.app.user("legacy").await,
        ctx.app.user("defaulted").await,
        ctx.app.user("chose").await,
        ctx.app.user("hid").await,
        ctx.app.user("adult").await,
        ctx.app.user("bot").await,
    );
    for user in [&legacy, &defaulted, &chose, &hid, &bot] {
        signed_up_without_birthdate(&ctx, user).await;
    }
    // A full stored by the default of its day, and the owners' own choices.
    sql(&ctx, "INSERT INTO notify_settings (user_id, push_preview) VALUES ($1, 1)", &defaulted).await;
    sql(&ctx, "INSERT INTO notify_settings (user_id, push_preview, push_preview_chosen) VALUES ($1, 1, true)", &chose).await;
    sql(&ctx, "INSERT INTO notify_settings (user_id, push_preview) VALUES ($1, 3)", &hid).await;
    // A bot goes by its owner, the adult.
    sqlx::query("UPDATE users SET bot = true, bot_owner_id = $2 WHERE id = $1")
        .bind(bot.id)
        .bind(adult.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();

    sqlx::raw_sql(include_str!("../../../migrations/1202_notify_push_preview_without_birthdate.sql"))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();

    for user in [&legacy, &defaulted, &chose, &hid] {
        gives_adult_birthdate(&ctx, user).await;
    }
    assert_eq!(preview(&ctx, &legacy).await, PushPreview::PUSH_PREVIEW_SENDER_ONLY, "as at sign-up");
    assert_eq!(preview(&ctx, &defaulted).await, PushPreview::PUSH_PREVIEW_SENDER_ONLY, "an old default is not a choice");
    assert_eq!(preview(&ctx, &chose).await, PushPreview::PUSH_PREVIEW_FULL, "their own choice stays");
    assert_eq!(preview(&ctx, &hid).await, PushPreview::PUSH_PREVIEW_HIDDEN, "only narrows");
    assert_eq!(preview(&ctx, &adult).await, PushPreview::PUSH_PREVIEW_FULL, "an adult's birthdate at sign-up");
    assert_eq!(preview(&ctx, &bot).await, PushPreview::PUSH_PREVIEW_FULL);
}
