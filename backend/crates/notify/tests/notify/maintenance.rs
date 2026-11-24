//! Retention, account deletion and the personal data export.
use tokuly_notify::events::{USER_DELETED_TOPIC, UserDeleted};
use tokuly_proto::notify::*;

use crate::common::*;

#[tokio::test]
async fn retention_removes_old_entries_and_stale_event_markers() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let db = &ctx.app.core.db;
    let fresh = ctx.deliver_to_inbox(&chat_message(&alice, &bob)).await;
    let aging = ctx.deliver_to_inbox(&chat_message(&alice, &bob)).await;
    let expired = ctx.deliver_to_inbox(&chat_message(&alice, &bob)).await;
    let age = |id: &String, days: i32| {
        let id = id.clone();
        async move {
            sqlx::query("UPDATE notifications SET occurred_at = now() - make_interval(days => $2) WHERE id = $1::uuid")
                .bind(id)
                .bind(days)
                .execute(db)
                .await
                .unwrap();
        }
    };
    age(&aging, 89).await;
    age(&expired, 91).await;
    // Markers of handled events: one recent, one older than the event stream keeps events.
    let (recent_marker, old_marker) = (uuid::Uuid::now_v7(), uuid::Uuid::now_v7());
    sqlx::query("INSERT INTO notify_processed_events (event_id, processed_at) VALUES ($1, now() - interval '2 days'), ($2, now() - interval '9 days')")
        .bind(recent_marker)
        .bind(old_marker)
        .execute(db)
        .await
        .unwrap();

    let report = ctx.notify.run_retention().await.unwrap();
    assert!(report.notifications >= 1 && report.processed_events >= 1, "{report:?}");

    let left: Vec<String> = ctx.inbox(&alice).await.notifications.into_iter().map(|n| n.id).collect();
    assert_eq!(left, [fresh, aging], "entries are kept for 90 days");
    let markers: Vec<uuid::Uuid> =
        sqlx::query_scalar("SELECT event_id FROM notify_processed_events WHERE event_id = ANY($1)")
            .bind(vec![recent_marker, old_marker])
            .fetch_all(db)
            .await
            .unwrap();
    assert_eq!(markers, [recent_marker]);
}

#[tokio::test]
async fn the_data_export_contains_the_users_notification_data_without_tokens() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    ctx.deliver_to_inbox(&chat_message(&alice, &bob)).await;
    ctx.deliver_to_inbox(&chat_message(&bob, &alice)).await;
    let _: UpdatePreferencesResponse = ctx
        .app
        .call(
            "tokuly.notify.v1.PreferencesService/UpdatePreferences",
            Some(&alice),
            &UpdatePreferencesRequest {
                categories: vec![CategoryChange { category: "live".into(), push: Some(false), ..Default::default() }],
                quiet_hours: QuietHours { enabled: true, start_minute: 1320, end_minute: 420, ..Default::default() }
                    .into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let _: MuteUserResponse = ctx
        .app
        .call(
            "tokuly.notify.v1.PreferencesService/MuteUser",
            Some(&alice),
            &MuteUserRequest { user_id: bob.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    let subscription = Subscription::new("https://fcm.googleapis.com/fcm/send/export-test-secret-endpoint");
    ctx.register(&alice, &subscription.request()).await;

    let exporter = ctx
        .app
        .core
        .exporters
        .all()
        .into_iter()
        .find(|e| e.name() == "notify")
        .expect("the notify exporter is registered");
    let export = exporter.export(alice.id).await.unwrap();

    let notifications = export["notifications"].as_array().unwrap();
    assert_eq!(notifications.len(), 1, "only the user's own inbox");
    assert_eq!(notifications[0]["title_ja"], "新着メッセージ");
    assert_eq!(notifications[0]["actor_id"], bob.id.to_string());
    assert_eq!(export["settings"]["quiet_enabled"], true);
    assert_eq!(
        export["category_preferences"],
        serde_json::json!([{ "category": "live", "inbox": true, "push": false }])
    );
    assert_eq!(export["muted_users"][0]["muted_user_id"], bob.id.to_string());
    let devices = export["push_devices"].as_array().unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0]["channel"], 1);
    assert!(
        !export.to_string().contains("export-test-secret-endpoint"),
        "push tokens are credentials, not exported data"
    );

    // Someone with nothing stored gets empty sections, not an error.
    let carol = ctx.app.user("carol").await;
    let empty = exporter.export(carol.id).await.unwrap();
    assert_eq!(empty["notifications"], serde_json::json!([]));
    assert_eq!(empty["settings"], serde_json::json!({}));
}

#[tokio::test]
async fn deleting_an_account_removes_its_notification_data() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await);
    let db = &ctx.app.core.db;
    // Alice has an inbox, preferences, a mute, a device...
    ctx.deliver_to_inbox(&chat_message(&alice, &bob)).await;
    update_preferences(&ctx, &alice, toggle("live", Some(false), Some(false))).await;
    let _: PausePushResponse = ctx
        .app
        .call("tokuly.notify.v1.PreferencesService/PausePush", Some(&alice), &PausePushRequest::default())
        .await
        .unwrap();
    let _: MuteUserResponse = ctx
        .app
        .call(
            "tokuly.notify.v1.PreferencesService/MuteUser",
            Some(&alice),
            &MuteUserRequest { user_id: carol.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    ctx.register_browser(&alice).await;
    // ...and shows up in Bob's data: a notification she caused and a mute on her.
    ctx.deliver_to_inbox(&chat_message(&bob, &alice)).await;
    let kept = ctx.deliver_to_inbox(&chat_message(&bob, &carol)).await;
    let _: MuteUserResponse = ctx
        .app
        .call(
            "tokuly.notify.v1.PreferencesService/MuteUser",
            Some(&bob),
            &MuteUserRequest { user_id: alice.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();

    let deleted = event(USER_DELETED_TOPIC, UserDeleted { user_id: alice.id });
    ctx.notify.handle_event(deleted.clone()).await.unwrap();
    ctx.notify.handle_event(deleted).await.unwrap(); // idempotent

    for table in
        ["notifications", "notify_settings", "notify_category_prefs", "notify_mutes", "notify_blocks", "push_devices"]
    {
        let rows: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table} WHERE user_id = $1")))
                .bind(alice.id)
                .fetch_one(db)
                .await
                .unwrap();
        assert_eq!(rows, 0, "{table}");
    }
    let bobs = ctx.inbox(&bob).await.notifications;
    assert_eq!(
        bobs.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
        [kept.as_str()],
        "what Alice caused is gone, the rest stays"
    );
    let muted: i64 = sqlx::query_scalar("SELECT count(*) FROM notify_mutes WHERE muted_user_id = $1")
        .bind(alice.id)
        .fetch_one(db)
        .await
        .unwrap();
    assert_eq!(muted, 0);
}

/// Rows of `notify_edits` and `notify_audiences` about `ids`.
async fn remembered(ctx: &Ctx, ids: &[uuid::Uuid]) -> (Vec<uuid::Uuid>, Vec<uuid::Uuid>) {
    let db = &ctx.app.core.db;
    let edits = sqlx::query_scalar("SELECT ref_id FROM notify_edits WHERE ref_id = ANY($1) ORDER BY ref_id")
        .bind(ids)
        .fetch_all(db)
        .await
        .unwrap();
    let audiences = sqlx::query_scalar("SELECT ref_id FROM notify_audiences WHERE ref_id = ANY($1) ORDER BY ref_id")
        .bind(ids)
        .fetch_all(db)
        .await
        .unwrap();
    (edits, audiences)
}

async fn remember_edit_and_audience(
    ctx: &Ctx,
    id: uuid::Uuid,
    author: uuid::Uuid,
    other: uuid::Uuid,
    owner: uuid::Uuid,
) {
    ctx.notify
        .handle_event(event(
            "engagement.comment.edited",
            serde_json::json!({
                "comment_id": id, "author_id": author, "body": { "ja": "x", "en": "x" }, "mentioned": [other],
                "owner_notice": { "user_id": owner, "kind": "engagement.comment", "title": { "ja": "t", "en": "t" } },
                "edited_at": chrono::Utc::now(),
            }),
        ))
        .await
        .unwrap();
    ctx.notify
        .handle_event(event(
            "engagement.comment.hidden",
            serde_json::json!({ "comment_id": id, "author_id": author, "visible_to": [other], "changed_at": chrono::Utc::now() }),
        ))
        .await
        .unwrap();
}

#[tokio::test]
async fn remembered_edits_and_audiences_are_swept_after_eight_days() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let (recent, old) = (uuid::Uuid::now_v7(), uuid::Uuid::now_v7());
    let (mut ids, owner) = (vec![recent, old], ctx.app.user("owner").await);
    ids.sort_unstable();
    for id in &ids {
        remember_edit_and_audience(&ctx, *id, alice.id, bob.id, owner.id).await;
    }
    assert_eq!(remembered(&ctx, &ids).await, (ids.clone(), ids.clone()));
    for table in ["notify_edits", "notify_audiences"] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE {table} SET recorded_at = now() - interval '9 days' WHERE ref_id = $1"
        )))
        .bind(old)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    }
    let report = ctx.notify.run_retention().await.unwrap();
    assert!(report.edits >= 1 && report.audiences >= 1, "{report:?}");
    assert_eq!(remembered(&ctx, &ids).await, (vec![recent], vec![recent]));
}

#[tokio::test]
async fn an_erased_account_leaves_no_remembered_edits_or_audiences() {
    let ctx = start().await;
    let (alice, bob, owner, carol) = (
        ctx.app.user("alice").await,
        ctx.app.user("bob").await,
        ctx.app.user("owner").await,
        ctx.app.user("carol").await,
    );
    let (by_alice, by_carol) = (uuid::Uuid::now_v7(), uuid::Uuid::now_v7());
    remember_edit_and_audience(&ctx, by_alice, alice.id, bob.id, owner.id).await;
    remember_edit_and_audience(&ctx, by_carol, carol.id, alice.id, alice.id).await;
    let erase = |user_id| event(USER_DELETED_TOPIC, UserDeleted { user_id });
    ctx.notify.handle_event(erase(alice.id)).await.unwrap();

    // Alice's comment is forgotten; Carol's no longer names Alice anywhere.
    assert_eq!(remembered(&ctx, &[by_alice, by_carol]).await, (vec![by_carol], vec![by_carol]));
    let (mentioned, owner_id): (Vec<uuid::Uuid>, Option<uuid::Uuid>) =
        sqlx::query_as("SELECT mentioned, owner_id FROM notify_edits WHERE ref_id = $1")
            .bind(by_carol)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert!(mentioned.is_empty() && owner_id.is_none());
    let visible_to: Vec<uuid::Uuid> = sqlx::query_scalar("SELECT visible_to FROM notify_audiences WHERE ref_id = $1")
        .bind(by_carol)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert!(visible_to.is_empty());
}
