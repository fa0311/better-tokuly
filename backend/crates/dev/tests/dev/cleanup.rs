//! Account deletion and the personal data export.
use tokuly_proto::dev::*;
use uuid::Uuid;

use crate::common::*;

#[tokio::test]
async fn a_deleted_account_leaves_nothing_behind() {
    let ctx = start().await;
    let (alice, dana, other) = (ctx.app.user("alice").await, ctx.app.user("dana").await, ctx.app.user("other").await);
    // Alice has a token, an application (which Other authorized), an
    // authorization of Dana's application, a webhook and a bot with a token.
    let (_, pat) = personal_token(&ctx, &alice, &["identity:read"]).await;
    let alices_app = create_app(&ctx, &alice, &["identity:read"], AppClientType::APP_CLIENT_TYPE_CONFIDENTIAL).await;
    let (others_access, _) = obtain_tokens(&ctx, &other, &alices_app, "identity:read").await;
    let danas_app = create_app(&ctx, &dana, &["identity:read"], AppClientType::APP_CLIENT_TYPE_CONFIDENTIAL).await;
    let (alices_access, _) = obtain_tokens(&ctx, &alice, &danas_app, "identity:read").await;
    create_webhook(&ctx, &alice, "http://127.0.0.1:9/hook", &["friend.added"]).await;
    let handle = format!("doomed_{}", tokuly_core::ids::random_base62(6).to_lowercase());
    let bot: CreateBotResponse = ctx
        .app
        .call(
            &format!("{BOT_SERVICE}/CreateBot"),
            Some(&alice),
            &CreateBotRequest { handle, display_name: "Doomed".into(), ..Default::default() },
        )
        .await
        .unwrap();
    let bot_id = Uuid::parse_str(&bot.bot.as_option().unwrap().user.as_option().unwrap().id).unwrap();
    for secret in [&pat, &others_access, &alices_access] {
        assert!(whoami(&ctx, &with_token(&alice, secret)).await.is_ok());
    }

    ctx.dev
        .on_account_event(&event("identity.user.deleted", serde_json::json!({ "user_id": alice.id })))
        .await
        .unwrap();

    let left: Vec<i64> = sqlx::query_scalar(
        "SELECT count(*) FROM dev_tokens WHERE user_id = $1 OR created_by = $1
         UNION ALL SELECT count(*) FROM dev_apps WHERE owner_id = $1
         UNION ALL SELECT count(*) FROM dev_authorizations WHERE user_id = $1 OR app_id = $2
         UNION ALL SELECT count(*) FROM dev_webhooks WHERE account_id = $1
         UNION ALL SELECT count(*) FROM dev_bots WHERE owner_id = $1 AND deleted_at IS NULL",
    )
    .bind(alice.id)
    .bind(Uuid::parse_str(&alices_app.id).unwrap())
    .fetch_all(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(left, [0, 0, 0, 0, 0]);
    // Every credential that hung on the account is dead, cached or not.
    for secret in [&pat, &others_access, &alices_access] {
        assert!(whoami(&ctx, &with_token(&alice, secret)).await.is_err());
    }
    // Her bot went with her; its account is left to identity's purge.
    assert!(tokuly_identity::users::get(&ctx.app.core, bot_id).await.unwrap().deletion_scheduled_at.is_some());
    // Dana's application itself is untouched.
    assert!(
        ctx.app
            .call::<_, GetAppResponse>(
                &format!("{APP_SERVICE}/GetApp"),
                Some(&dana),
                &GetAppRequest { app_id: danas_app.id.clone(), ..Default::default() }
            )
            .await
            .is_ok()
    );
    // Handling the event again is harmless.
    ctx.dev
        .on_account_event(&event("identity.user.deleted", serde_json::json!({ "user_id": alice.id })))
        .await
        .unwrap();
}

#[tokio::test]
async fn the_data_export_lists_everything_but_never_a_secret() {
    let ctx = start().await;
    let (alice, dana) = (ctx.app.user("alice").await, ctx.app.user("dana").await);
    let (_, pat) = personal_token(&ctx, &alice, &["identity:read"]).await;
    let app = create_app(&ctx, &alice, &["identity:read"], AppClientType::APP_CLIENT_TYPE_CONFIDENTIAL).await;
    let danas_app = create_app(&ctx, &dana, &["notes:read"], AppClientType::APP_CLIENT_TYPE_CONFIDENTIAL).await;
    let (access, refresh) = obtain_tokens(&ctx, &alice, &danas_app, "notes:read").await;
    let (_, webhook_secret) = create_webhook(&ctx, &alice, "http://127.0.0.1:9/hook", &["friend.added"]).await;
    let handle = format!("exported_{}", tokuly_core::ids::random_base62(6).to_lowercase());
    ctx.app
        .call::<_, CreateBotResponse>(
            &format!("{BOT_SERVICE}/CreateBot"),
            Some(&alice),
            &CreateBotRequest { handle: handle.clone(), display_name: "Exported".into(), ..Default::default() },
        )
        .await
        .unwrap();

    let exporter =
        ctx.app.core.exporters.all().into_iter().find(|e| e.name() == "dev").expect("the dev exporter is registered");
    let export = exporter.export(alice.id).await.unwrap();
    assert_eq!(export["tokens"][0]["name"], "test token");
    assert_eq!(export["applications"][0]["id"], app.id);
    assert_eq!(
        (
            export["authorized_applications"][0]["app_name"].as_str(),
            export["authorized_applications"][0]["scopes"][0].as_str()
        ),
        (Some("Test App"), Some("notes:read"))
    );
    assert_eq!(export["bots"][0]["handle"], handle);
    assert_eq!(export["webhooks"][0]["url"], "http://127.0.0.1:9/hook");
    let text = export.to_string();
    for secret in [&pat, &app.secret, &access, &refresh, &webhook_secret] {
        assert!(!text.contains(secret.as_str()));
    }
    assert!(!text.contains("hash") && !text.contains("sealed"), "not even in hashed form");
    // Somebody with nothing exports empty sections.
    let empty = exporter.export(dana.id).await.unwrap();
    assert_eq!(empty["tokens"], serde_json::json!([]));
    assert_eq!(empty["applications"].as_array().unwrap().len(), 1);
}
