//! Migration 0601: accounts that signed up before the birthdate question keep
//! the minors' AI-consent default their sign-up gave them. A test binary of
//! its own: the migration touches every account of the database.
mod common;

use common::*;
use tokuly_proto::chat::*;
use tokuly_testkit::TestUser;

async fn sql(ctx: &Ctx, query: &'static str, user: &TestUser) {
    sqlx::query(query).bind(user.id).execute(&ctx.app.core.db).await.unwrap();
}

/// Signed up two years ago, before the question: no birthdate.
async fn signed_up_without_birthdate(ctx: &Ctx, user: &TestUser) {
    sql(ctx, "UPDATE users SET birthdate = NULL, created_at = now() - interval '2 years' WHERE id = $1", user).await;
}

/// `SetBirthdate` with an adult's date (`identity.user.birthdate_given` with
/// `minor: false` changes nothing in chat).
async fn gives_adult_birthdate(ctx: &Ctx, user: &TestUser) {
    sql(ctx, "UPDATE users SET birthdate = current_date - interval '30 years' WHERE id = $1", user).await;
}

async fn settings_of(ctx: &Ctx, user: &TestUser) -> AssistSettings {
    let response: GetAssistSettingsResponse =
        ctx.call("Assist", "GetAssistSettings", user, &GetAssistSettingsRequest::default()).await.unwrap();
    response.settings.into_option().expect("settings")
}

/// No birthdate at sign-up counts as a minor. Once one is given, a read that
/// finds nothing stored (`identity.user.created` lost) could not tell it was
/// not there at sign-up: the migration stores the minors' default while the
/// absence is still visible.
#[tokio::test]
async fn accounts_without_a_birthdate_keep_the_minors_consent_default() {
    let ctx = start().await;
    let (legacy, unstored, chose, adult, bot, deleted) = (
        ctx.app.user("legacy").await,
        ctx.app.user("unstored").await,
        ctx.app.user("chose").await,
        ctx.app.user("adult").await,
        ctx.app.user("bot").await,
        ctx.app.user("deleted").await,
    );
    for user in [&legacy, &unstored, &chose, &bot, &deleted] {
        signed_up_without_birthdate(&ctx, user).await;
    }
    // A row whose default was never filled (a lost event), and the owner's own choice.
    sql(&ctx, "INSERT INTO chat_ai_user_settings (user_id, provider) VALUES ($1, 0)", &unstored).await;
    sql(
        &ctx,
        "INSERT INTO chat_ai_user_settings (user_id, allow_message_use, message_use_chosen) VALUES ($1, true, true)",
        &chose,
    )
    .await;
    sqlx::query("UPDATE users SET bot = true, bot_owner_id = $2 WHERE id = $1")
        .bind(bot.id)
        .bind(adult.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    sql(&ctx, "UPDATE users SET deleted_at = now() WHERE id = $1", &deleted).await;

    sqlx::raw_sql(include_str!("../../../migrations/0601_chat_message_use_without_birthdate.sql"))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();

    for user in [&legacy, &unstored, &chose] {
        gives_adult_birthdate(&ctx, user).await;
    }
    for user in [&legacy, &unstored] {
        let settings = settings_of(&ctx, user).await;
        assert!(!settings.allow_message_use && settings.allow_message_use_is_default, "as at sign-up");
    }
    let settings = settings_of(&ctx, &chose).await;
    assert!(settings.allow_message_use && !settings.allow_message_use_is_default, "their own choice stays");
    assert!(settings_of(&ctx, &adult).await.allow_message_use, "an adult's birthdate at sign-up");
    let untouched: i64 = sqlx::query_scalar("SELECT count(*) FROM chat_ai_user_settings WHERE user_id = ANY($1)")
        .bind(vec![bot.id, deleted.id])
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(untouched, 0, "bots and deleted accounts are left alone");
}
