//! The background workers, end to end. Nothing is run by hand here: the
//! test starts what a worker process runs — the outbox relay, the media
//! workers and the emoji workers — and watches the work arrive.
mod common;

use std::time::Duration;

use common::{pack::*, *};
use tokuly_emoji::{EmojiConfig, catalog};
use tokuly_proto::emoji::*;

/// Polls until `check` finds what it waits for. The machine may be busy, so
/// the patience is generous.
async fn eventually<T>(what: &str, mut check: impl AsyncFnMut() -> Option<T>) -> T {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        if let Some(found) = check().await {
            return found;
        }
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[tokio::test]
async fn the_workers_carry_imports_exports_and_use_counts_through() {
    let ctx = start_with(EmojiConfig { usage_flush_seconds: 5, ..Default::default() }).await;
    let core = &ctx.app.core;
    core.storage.ensure_buckets().await.unwrap();
    tokio::spawn(tokuly_core::events::run_outbox_relay(core.db.clone(), core.bus.clone(), core.shutdown.clone()));
    ctx.media.spawn_workers();
    ctx.emoji.spawn_workers();
    let moderator = ctx.moderator().await;
    let mut topic = ctx.app.listen_topic(catalog::TOPIC).await;

    // An import: the job reads the pack, media processes the pictures, the
    // events of the finished uploads turn them into emoji.
    let t = unique("");
    let (still, moving) = (format!("still{t}"), format!("moving{t}"));
    let records = vec![
        record(&still, "still.png", serde_json::json!({ "aliases": ["いちまいえ"] })),
        record(&moving, "moving.gif", serde_json::json!({ "type": "image/gif" })),
        record(&format!("missing{t}"), "missing.png", serde_json::json!({})),
    ];
    let archive = zip_of(&[("meta.json", &meta(records)), ("still.png", &png(48)), ("moving.gif", &gif(32))]);
    let archive = upload_archive(&ctx, &moderator, archive).await;
    let import = start_import(&ctx, &moderator, &archive, ImportConflictPolicy::IMPORT_CONFLICT_POLICY_SKIP, "").await;
    let done = eventually("the import to complete", async || {
        let import = get_import(&ctx, &moderator, &import.id).await;
        (import.import.status == ImportStatus::IMPORT_STATUS_COMPLETED).then_some(import)
    })
    .await;
    assert_eq!((done.import.imported_count, done.import.failed_count), (2, 1));
    assert!(catalog::exists(core, &still).await.unwrap());
    assert!(ctx.get(None, &moving).await.unwrap().emoji.animated);
    let version = ctx.catalog().await.version;
    let announced = next_matching(&mut topic, |frame| match emoji_event(frame)? {
        tokuly_proto::emoji::__buffa::oneof::emoji_event::Event::CatalogChanged(changed)
            if changed.version >= version =>
        {
            Some(())
        }
        _ => None,
    })
    .await;
    assert!(announced.is_some(), "clients hear of the imported emoji");

    // An export: the job writes the archive.
    let request = StartExportRequest::default();
    let export: StartExportResponse = ctx.call("Admin", "StartExport", Some(&moderator), &request).await.unwrap();
    let ready = eventually("the export to be ready", async || {
        let request = GetExportRequest { export_id: export.export.id.clone(), ..Default::default() };
        let export: GetExportResponse = ctx.call("Admin", "GetExport", Some(&moderator), &request).await.unwrap();
        (export.export.status == ExportStatus::EXPORT_STATUS_READY).then(|| export.export.into_option().unwrap())
    })
    .await;
    assert!(ready.emoji_count >= 2 && !ready.download_url.is_empty());

    // Use: counted in the cache, written by the usage worker.
    for n in 0..3 {
        ctx.record_use(&ctx.app.user(&format!("fan{n}")).await, &[&still]).await;
    }
    eventually("the uses to be counted", async || {
        (ctx.get(Some(&moderator), &still).await.unwrap().use_count == 3).then_some(())
    })
    .await;
    eventually("the emoji to become popular", async || {
        let request = ListPopularEmojiRequest { limit: 100, ..Default::default() };
        let popular: ListPopularEmojiResponse = ctx.call("", "ListPopularEmoji", None, &request).await.unwrap();
        named(&popular.emojis, &still).is_some().then_some(())
    })
    .await;

    core.shutdown.cancel();
}

/// The Valkey key holding the catalogue's `what` for this test database.
async fn catalog_key(ctx: &Ctx, what: &str) -> String {
    let epoch: uuid::Uuid =
        sqlx::query_scalar("SELECT epoch FROM emoji_catalog").fetch_one(&ctx.app.core.db).await.unwrap();
    format!("emoji:{}:catalog:{what}", epoch.simple())
}

#[tokio::test]
async fn removals_are_forgotten_after_their_retention() {
    let ctx = start().await;
    let moderator = ctx.moderator().await;
    let (gone, stays) = (ctx.add(&moderator, &unique("gone")).await, ctx.add(&moderator, &unique("stays")).await);
    let before = ctx.catalog().await.version;
    ctx.delete(&moderator, &gone.emoji.id).await;
    let after_removal = ctx.catalog().await.version;
    assert!(ctx.changes(before).await.removed_ids.contains(&gone.emoji.id));

    // Within the retention nothing is forgotten.
    ctx.emoji.prune().await.unwrap();
    ctx.add(&moderator, &unique("fresh")).await;
    let remembered = ctx.changes(before).await;
    assert!(!remembered.full && remembered.removed_ids.contains(&gone.emoji.id));

    // Past it, the removal goes; a client from before it starts over.
    sqlx::query("UPDATE emoji_catalog_removals SET removed_at = now() - interval '91 days' WHERE emoji_id = $1::uuid")
        .bind(&gone.emoji.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.emoji.prune().await.unwrap();
    let later = ctx.add(&moderator, &unique("later")).await;
    let restart = ctx.changes(before).await;
    assert!(restart.full, "a difference could no longer say that the emoji is gone");
    assert!(
        named(&restart.upserted, &stays.emoji.name).is_some() && named(&restart.upserted, &gone.emoji.name).is_none()
    );
    // A client that had already seen the removal still gets a difference.
    let recent = ctx.changes(after_removal).await;
    assert!(!recent.full && named(&recent.upserted, &later.emoji.name).is_some());
    assert!(!recent.removed_ids.contains(&gone.emoji.id));
}

#[tokio::test]
async fn a_cache_ahead_of_a_restored_database_is_taken_back() {
    let ctx = start().await;
    let core = &ctx.app.core;
    let moderator = ctx.moderator().await;
    ctx.add(&moderator, &unique("before")).await;
    let version: i64 = sqlx::query_scalar("SELECT version FROM emoji_catalog").fetch_one(&core.db).await.unwrap();

    // Valkey remembers versions this database does not have (any more).
    let mut valkey = core.cache.conn();
    for what in ["version", "announced"] {
        redis::cmd("SET")
            .arg(catalog_key(&ctx, what).await)
            .arg(version + 1000)
            .query_async::<()>(&mut valkey)
            .await
            .unwrap();
    }
    // Readers are not fooled meanwhile: what they serve is the database's.
    assert!(ctx.catalog().await.version < version + 1000);

    ctx.emoji.maintain().await.unwrap();
    let cached: i64 = redis::cmd("GET").arg(catalog_key(&ctx, "version").await).query_async(&mut valkey).await.unwrap();
    assert!(cached < version + 1000, "the cache is back at the database's version");
    // And changes are announced again, although Valkey claimed to have
    // announced far later versions.
    let mut topic = ctx.app.listen_topic(catalog::TOPIC).await;
    let added = ctx.add(&moderator, &unique("after")).await;
    let announced = next_matching(&mut topic, |frame| match emoji_event(frame)? {
        tokuly_proto::emoji::__buffa::oneof::emoji_event::Event::CatalogChanged(changed) => Some(changed.version),
        _ => None,
    })
    .await
    .expect("the change is announced");
    assert!(announced < version + 1000);
    assert!(named(&ctx.catalog().await.emojis, &added.emoji.name).is_some());
}
