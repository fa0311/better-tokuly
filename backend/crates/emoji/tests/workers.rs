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
    let fan = ctx.app.user("fan").await;
    ctx.record_use(&fan, &[&still]).await;
    eventually("the use to be counted", async || (ctx.get(None, &still).await.unwrap().use_count == 1).then_some(()))
        .await;
    let popular: ListPopularEmojiResponse = ctx
        .call("", "ListPopularEmoji", None, &ListPopularEmojiRequest { limit: 100, ..Default::default() })
        .await
        .unwrap();
    assert!(named(&popular.emojis, &still).is_some());

    core.shutdown.cancel();
}
