//! Importing and exporting emoji packs in Misskey's format.
//!
//! The packs here are built by hand (see `common::pack`), their pictures
//! are real images, and the media domain processes them for real:
//! `process_pictures` does what its worker does.
mod common;

use std::io::{Cursor, Read as _};

use common::{pack::*, *};
use tokuly_core::storage::Bucket;
use tokuly_emoji::{EmojiConfig, catalog};
use tokuly_proto::emoji::{__buffa::oneof::emoji_event, *};
use tokuly_testkit::TestUser;
use uuid::Uuid;
use zip::ZipArchive;

async fn run_import(ctx: &Ctx, import_id: &str) {
    ctx.emoji.run_import(import_id.parse().unwrap()).await.expect("the import job");
}

/// The uploads made from an import's pictures so far.
async fn picture_uploads(ctx: &Ctx, import_id: &str) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT asset_id FROM emoji_import_entries WHERE import_id = $1::uuid AND asset_id IS NOT NULL ORDER BY position")
        .bind(import_id)
        .fetch_all(&ctx.app.core.db)
        .await
        .unwrap()
}

/// What the media worker does with an import's pictures: processes each.
async fn process_pictures(ctx: &Ctx, import_id: &str) -> Vec<Uuid> {
    let uploads = picture_uploads(ctx, import_id).await;
    for upload in &uploads {
        ctx.media.process_now(*upload).await.expect("media processing");
    }
    uploads
}

/// Delivers the events the media domain emitted for these uploads.
async fn deliver_media_events(ctx: &Ctx, uploads: &[Uuid]) {
    for event in ctx.app.events("media.asset.").await {
        let about = event.data["asset_id"].as_str().and_then(|id| id.parse::<Uuid>().ok());
        if about.is_some_and(|id| uploads.contains(&id)) {
            ctx.emoji.handle_event(event).await.expect("media event");
        }
    }
}

#[tokio::test]
async fn a_misskey_pack_becomes_emoji_entry_by_entry() {
    let ctx = start().await;
    ctx.app.core.storage.ensure_buckets().await.unwrap();
    let moderator = ctx.moderator().await;
    let t = unique("");
    let taken = format!("taken{t}");
    ctx.add(&moderator, &taken).await;
    let category = format!("Blobs{t}");

    let (cat, party, broken) = (format!("Blob-Cat{t}"), format!("party{t}"), format!("broken{t}"));
    let records = vec![
        record(
            &cat,
            "blobcat.png",
            serde_json::json!({
                "category": category, "aliases": ["Cat", "ねこ", "two words"], "license": "CC0", "isSensitive": true,
            }),
        ),
        record(
            &party,
            "party.gif",
            serde_json::json!({ "category": category, "type": "image/gif", "localOnly": true }),
        ),
        record(&taken, "taken.png", serde_json::json!({})),
        serde_json::json!({ "fileName": "gone.png", "downloaded": false, "emoji": { "name": format!("gone{t}") } }),
        record(&format!("missing{t}"), "missing.png", serde_json::json!({})),
        record("ねこ", "neko.png", serde_json::json!({})),
        record(&format!("text{t}"), "notes.png", serde_json::json!({})),
        record(&broken, "broken.png", serde_json::json!({})),
        record(&format!("escape{t}"), "../evil.png", serde_json::json!({})),
        record(&format!("blob_cat{t}"), "blobcat.png", serde_json::json!({})),
    ];
    let mut broken_png = png(32);
    broken_png.truncate(60);
    let archive = zip_of(&[
        ("meta.json", &meta(records)),
        ("blobcat.png", &png(64)),
        ("party.gif", &gif(48)),
        ("taken.png", &png(32)),
        ("neko.png", &png(32)),
        ("notes.png", b"these are notes, not a picture"),
        ("broken.png", &broken_png),
        ("../evil.png", &png(32)),
    ]);
    let archive = upload_archive(&ctx, &moderator, archive).await;

    let mut inbox = ctx.app.listen_user(moderator.id).await;
    let queued =
        start_import(&ctx, &moderator, &archive, ImportConflictPolicy::IMPORT_CONFLICT_POLICY_UNSPECIFIED, "").await;
    assert_eq!(queued.status, ImportStatus::IMPORT_STATUS_QUEUED);
    assert_eq!(queued.on_conflict, ImportConflictPolicy::IMPORT_CONFLICT_POLICY_SKIP, "skipping is the default");
    assert_eq!(queued.requested_by.id, moderator.id.to_string());
    assert_eq!(ctx.holders(&archive).await, 1, "the import keeps the archive while it runs");

    // The job reads the pack: every entry gets a plan, the pictures go to media.
    run_import(&ctx, &queued.id).await;
    let reading = get_import(&ctx, &moderator, &queued.id).await;
    assert_eq!(reading.import.status, ImportStatus::IMPORT_STATUS_RUNNING);
    assert_eq!((reading.import.total_count, reading.import.source_host.as_str()), (10, "misskey.example"));
    assert_eq!(reading.entries.len(), 10);
    let reasons: Vec<(&str, &str)> =
        reading.entries.iter().map(|e| (e.source_name.as_str(), e.reason.as_str())).collect();
    assert_eq!(
        reasons,
        [
            (cat.as_str(), ""),
            (party.as_str(), ""),
            (taken.as_str(), "NAME_TAKEN"),
            (format!("gone{t}").as_str(), "NOT_DOWNLOADED"),
            (format!("missing{t}").as_str(), "FILE_MISSING"),
            ("ねこ", "NAME_INVALID"),
            (format!("text{t}").as_str(), "UNSUPPORTED_CONTENT"),
            (broken.as_str(), ""),
            (format!("escape{t}").as_str(), "FILE_NAME_INVALID"),
            (format!("blob_cat{t}").as_str(), "DUPLICATE_NAME"),
        ]
    );
    for waiting in [&cat, &party, &broken] {
        assert_eq!(entry(&reading, waiting).status, ImportEntryStatus::IMPORT_ENTRY_STATUS_PROCESSING);
    }
    assert_eq!(entry(&reading, &taken).status, ImportEntryStatus::IMPORT_ENTRY_STATUS_SKIPPED);
    assert_eq!(entry(&reading, "ねこ").status, ImportEntryStatus::IMPORT_ENTRY_STATUS_FAILED);
    assert_eq!(entry(&reading, "ねこ").name, "");
    assert!(
        !catalog::exists(&ctx.app.core, &format!("blob_cat{t}")).await.unwrap(),
        "nothing is usable before its picture is"
    );
    // Running the job again changes nothing.
    run_import(&ctx, &queued.id).await;
    assert_eq!(picture_uploads(&ctx, &queued.id).await.len(), 3);

    // Media processes the pictures; its events settle the entries.
    let uploads = process_pictures(&ctx, &queued.id).await;
    deliver_media_events(&ctx, &uploads).await;
    let done = get_import(&ctx, &moderator, &queued.id).await;
    assert_eq!(done.import.status, ImportStatus::IMPORT_STATUS_COMPLETED);
    assert!(done.import.completed_at.is_set() && done.import.error.is_empty());
    assert_eq!((done.import.imported_count, done.import.skipped_count, done.import.failed_count), (2, 2, 6));
    assert_eq!(ctx.holders(&archive).await, 0, "the archive is let go");

    let imported = entry(&done, &cat);
    assert_eq!(imported.status, ImportEntryStatus::IMPORT_ENTRY_STATUS_IMPORTED);
    assert_eq!(imported.name, format!("blob_cat{t}"), "the name is brought into our alphabet");
    let failed = entry(&done, &broken);
    assert_eq!(failed.status, ImportEntryStatus::IMPORT_ENTRY_STATUS_FAILED);
    assert!(!failed.reason.is_empty() && failed.emoji_id.is_empty(), "media's reason is passed on: {}", failed.reason);

    // The emoji are in the catalogue as the pack describes them.
    let blobcat = ctx.get(Some(&moderator), &imported.emoji_id).await.unwrap();
    assert_eq!(blobcat.state, EmojiState::EMOJI_STATE_APPROVED);
    assert_eq!(blobcat.emoji.name, format!("blob_cat{t}"));
    assert_eq!((blobcat.emoji.category.as_str(), blobcat.license.as_str()), (category.as_str(), "CC0"));
    assert_eq!(blobcat.emoji.aliases, strings(&["cat", "ねこ"]), "what cannot be an alias is left out");
    assert!(blobcat.emoji.sensitive && !blobcat.emoji.local_only && !blobcat.emoji.animated);
    assert_eq!((blobcat.emoji.image.width, blobcat.emoji.image.height), (64, 64));
    assert!(blobcat.emoji.image.url.contains("/img/") && !blobcat.emoji.image.variants.is_empty());
    assert!(!blobcat.author.is_set(), "nobody here made it");
    assert_eq!(blobcat.history.len(), 1);
    assert_eq!(blobcat.history[0].action, ReviewAction::REVIEW_ACTION_IMPORTED);
    assert_eq!(blobcat.history[0].actor.id, moderator.id.to_string());
    let animated = ctx.get(None, &party).await.unwrap();
    assert!(animated.emoji.animated && animated.emoji.local_only);
    assert!(animated.emoji.image.url.contains("/anim/"), "the animation is kept: {}", animated.emoji.image.url);
    assert_eq!(animated.emoji.category, category);
    let catalogue = ctx.catalog().await;
    assert!(named(&catalogue.emojis, &party).is_some());
    assert_eq!(
        catalogue.categories.iter().find(|c| c.name == category).expect("the pack's category was created").emoji_count,
        2
    );
    let added = ctx.emoji_events("emoji.emoji.added", &imported.emoji_id).await;
    assert_eq!((added.len(), added[0]["via"].as_str()), (1, Some("import")));

    // The moderator's screen followed along.
    let finished = next_matching(&mut inbox, |frame| match emoji_event(frame)? {
        emoji_event::Event::ImportUpdated(import)
            if import.id == queued.id && import.status == ImportStatus::IMPORT_STATUS_COMPLETED =>
        {
            Some(import.imported_count)
        }
        _ => None,
    })
    .await;
    assert_eq!(finished, Some(2));

    // Entries page; imports are listed newest first.
    let request = GetImportRequest { import_id: queued.id.clone(), page: page(4, ""), ..Default::default() };
    let first: GetImportResponse = ctx.call("Admin", "GetImport", Some(&moderator), &request).await.unwrap();
    assert_eq!(first.entries.len(), 4);
    assert!(first.page.has_more);
    let request = GetImportRequest { page: page(100, &first.page.next_cursor), ..request };
    let rest: GetImportResponse = ctx.call("Admin", "GetImport", Some(&moderator), &request).await.unwrap();
    assert_eq!((rest.entries.len(), rest.entries[0].source_name.as_str()), (6, format!("missing{t}").as_str()));
    let listed: ListImportsResponse =
        ctx.call("Admin", "ListImports", Some(&moderator), &ListImportsRequest::default()).await.unwrap();
    assert!(listed.imports.iter().any(|import| import.id == queued.id));
}

#[tokio::test]
async fn clashing_names_are_numbered_when_asked_even_when_the_clash_comes_late() {
    let ctx = start().await;
    ctx.app.core.storage.ensure_buckets().await.unwrap();
    let moderator = ctx.moderator().await;
    let t = unique("");
    let (dup, late) = (format!("dup{t}"), format!("late{t}"));
    ctx.add(&moderator, &dup).await;
    let shelf = format!("まとめ{t}");
    let pack = || {
        zip_of(&[
            (
                "meta.json",
                &meta(vec![
                    record(&dup, "dup.png", serde_json::json!({ "category": "ignored" })),
                    record(&late, "late.png", serde_json::json!({})),
                ]),
            ),
            ("dup.png", &png(40)),
            ("late.png", &png(40)),
        ])
    };

    // Renaming: the taken name gets the next free number.
    let archive = upload_archive(&ctx, &moderator, pack()).await;
    let renaming =
        start_import(&ctx, &moderator, &archive, ImportConflictPolicy::IMPORT_CONFLICT_POLICY_RENAME, &shelf).await;
    assert_eq!(renaming.category, shelf);
    run_import(&ctx, &renaming.id).await;
    let planned = get_import(&ctx, &moderator, &renaming.id).await;
    assert_eq!(entry(&planned, &dup).name, format!("{dup}_2"));
    // While the pictures are processed, somebody takes both planned names.
    ctx.add(&moderator, &format!("{dup}_2")).await;
    ctx.add(&moderator, &late).await;
    process_pictures(&ctx, &renaming.id).await;
    // No event this time: the import is settled by a later pass.
    ctx.emoji.settle_import(renaming.id.parse().unwrap()).await.unwrap();
    let done = get_import(&ctx, &moderator, &renaming.id).await;
    assert_eq!(done.import.status, ImportStatus::IMPORT_STATUS_COMPLETED);
    assert_eq!((done.import.imported_count, done.import.skipped_count, done.import.failed_count), (2, 0, 0));
    for (source, name) in [(&dup, format!("{dup}_3")), (&late, format!("{late}_2"))] {
        let renamed = entry(&done, source);
        assert_eq!(renamed.status, ImportEntryStatus::IMPORT_ENTRY_STATUS_RENAMED);
        assert_eq!(renamed.name, name);
        let emoji = ctx.get(None, &name).await.unwrap();
        assert_eq!(emoji.emoji.id, renamed.emoji_id);
        assert_eq!(emoji.emoji.category, shelf, "the import's category wins over the pack's");
    }
    assert!(ctx.catalog().await.categories.iter().all(|c| c.name != "ignored"));

    // Skipping: the same pack again leaves everything as it is, and a name
    // taken while a picture is processed is skipped as well.
    let fresh = format!("fresh{t}");
    let archive = zip_of(&[
        (
            "meta.json",
            &meta(vec![
                record(&dup, "dup.png", serde_json::json!({})),
                record(&fresh, "fresh.png", serde_json::json!({})),
            ]),
        ),
        ("dup.png", &png(40)),
        ("fresh.png", &png(40)),
    ]);
    let archive = upload_archive(&ctx, &moderator, archive).await;
    let skipping =
        start_import(&ctx, &moderator, &archive, ImportConflictPolicy::IMPORT_CONFLICT_POLICY_SKIP, "").await;
    run_import(&ctx, &skipping.id).await;
    let late_clash = ctx.add(&moderator, &fresh).await;
    let uploads = process_pictures(&ctx, &skipping.id).await;
    deliver_media_events(&ctx, &uploads).await;
    let done = get_import(&ctx, &moderator, &skipping.id).await;
    assert_eq!(done.import.status, ImportStatus::IMPORT_STATUS_COMPLETED);
    assert_eq!((done.import.imported_count, done.import.skipped_count), (0, 2));
    for source in [&dup, &fresh] {
        let skipped = entry(&done, source);
        assert_eq!(
            (skipped.status, skipped.reason.as_str()),
            (ImportEntryStatus::IMPORT_ENTRY_STATUS_SKIPPED.into(), "NAME_TAKEN")
        );
    }
    assert_eq!(ctx.get(None, &fresh).await.unwrap().emoji.id, late_clash.emoji.id, "the existing emoji is untouched");
    // The picture nobody will use is deleted rather than left lying around.
    let leftover: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT deleted_at FROM media_assets WHERE id = $1")
            .bind(uploads[0])
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert!(leftover.is_some());
}

#[tokio::test]
async fn a_pack_that_cannot_be_read_fails_as_a_whole() {
    let ctx = start_with(EmojiConfig { import_max_entries: 2, ..Default::default() }).await;
    ctx.app.core.storage.ensure_buckets().await.unwrap();
    let (moderator, other) = (ctx.moderator().await, ctx.moderator().await);
    let three = vec![
        record(&unique("a"), "a.png", serde_json::json!({})),
        record(&unique("b"), "a.png", serde_json::json!({})),
        record(&unique("c"), "a.png", serde_json::json!({})),
    ];
    let unreadable: [(Vec<u8>, &str); 5] = [
        (png(32), "ARCHIVE_INVALID"),
        (zip_of(&[("a.png", &png(32))]), "META_MISSING"),
        (zip_of(&[("meta.json", b"{ not json"), ("a.png", &png(32))]), "META_INVALID"),
        (zip_of(&[("meta.json", br#"{"host":"x"}"#)]), "META_INVALID"),
        (zip_of(&[("meta.json", &meta(three)), ("a.png", &png(32))]), "TOO_MANY_ENTRIES"),
    ];
    for (bytes, reason) in unreadable {
        let archive = upload_archive(&ctx, &moderator, bytes).await;
        let import =
            start_import(&ctx, &moderator, &archive, ImportConflictPolicy::IMPORT_CONFLICT_POLICY_SKIP, "").await;
        let mut inbox = ctx.app.listen_user(moderator.id).await;
        run_import(&ctx, &import.id).await;
        let failed = get_import(&ctx, &moderator, &import.id).await;
        assert_eq!(
            (failed.import.status, failed.import.error.as_str()),
            (ImportStatus::IMPORT_STATUS_FAILED.into(), reason)
        );
        assert!(failed.entries.is_empty() && failed.import.completed_at.is_set());
        assert_eq!(ctx.holders(&archive).await, 0);
        let told = next_matching(&mut inbox, |frame| match emoji_event(frame)? {
            emoji_event::Event::ImportUpdated(update) if update.id == import.id => Some(update.error.clone()),
            _ => None,
        })
        .await;
        assert_eq!(told.as_deref(), Some(reason));
        // A failed import stays failed.
        run_import(&ctx, &import.id).await;
        assert_eq!(get_import(&ctx, &moderator, &import.id).await.import.status, ImportStatus::IMPORT_STATUS_FAILED);
    }

    // What is refused before an import even starts.
    let archive = upload_archive(&ctx, &moderator, zip_of(&[("meta.json", br#"{"emojis":[]}"#)])).await;
    let request = StartImportRequest { asset_id: archive.clone(), ..Default::default() };
    assert_eq!(
        ctx.reason("Admin", "StartImport", Some(&other), &request).await,
        "ASSET_NOT_FOUND",
        "another moderator's upload"
    );
    let wrong = StartImportRequest { asset_id: ctx.picture(&moderator).await, ..Default::default() };
    assert_eq!(ctx.reason("Admin", "StartImport", Some(&moderator), &wrong).await, "ASSET_WRONG_PURPOSE");
    let unknown = StartImportRequest { asset_id: Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(ctx.reason("Admin", "StartImport", Some(&moderator), &unknown).await, "ASSET_NOT_FOUND");
    let malformed = StartImportRequest { asset_id: "nope".to_owned(), ..Default::default() };
    assert_eq!(ctx.reason("Admin", "StartImport", Some(&moderator), &malformed).await, "INVALID_ID");
    let bad_category = StartImportRequest { category: "/".to_owned(), ..request.clone() };
    assert_eq!(ctx.reason("Admin", "StartImport", Some(&moderator), &bad_category).await, "CATEGORY_NAME_INVALID");
    sqlx::query("UPDATE media_assets SET size_bytes = 100 * 1024 * 1024 + 1 WHERE id = $1::uuid")
        .bind(&archive)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(ctx.reason("Admin", "StartImport", Some(&moderator), &request).await, "ARCHIVE_TOO_LARGE");
    let missing = GetImportRequest { import_id: Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(ctx.reason("Admin", "GetImport", Some(&moderator), &missing).await, "IMPORT_NOT_FOUND");

    // An empty pack is a finished import of nothing.
    let empty = upload_archive(&ctx, &moderator, zip_of(&[("meta.json", br#"{"emojis":[]}"#)])).await;
    let import = start_import(&ctx, &moderator, &empty, ImportConflictPolicy::IMPORT_CONFLICT_POLICY_SKIP, "").await;
    run_import(&ctx, &import.id).await;
    let done = get_import(&ctx, &moderator, &import.id).await;
    assert_eq!((done.import.status, done.import.total_count), (ImportStatus::IMPORT_STATUS_COMPLETED.into(), 0));
}

/// Makes a fixture upload look like a real one: bytes in storage, and the
/// content type they have.
async fn store_original(ctx: &Ctx, asset_id: &str, bytes: &[u8], content_type: &str) {
    let key: String =
        sqlx::query_scalar("UPDATE media_assets SET content_type = $2 WHERE id = $1::uuid RETURNING original_key")
            .bind(asset_id)
            .bind(content_type)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    ctx.app.core.storage.put(Bucket::Private, &key, bytes.to_vec(), content_type).await.unwrap();
}

async fn start_export(ctx: &Ctx, moderator: &TestUser, category: Option<&str>) -> Export {
    let request = StartExportRequest { category: category.map(str::to_owned), ..Default::default() };
    let response: StartExportResponse = ctx.call("Admin", "StartExport", Some(moderator), &request).await.unwrap();
    response.export.into_option().unwrap()
}

async fn get_export(ctx: &Ctx, moderator: &TestUser, export_id: &str) -> Export {
    let request = GetExportRequest { export_id: export_id.to_owned(), ..Default::default() };
    let response: GetExportResponse = ctx.call("Admin", "GetExport", Some(moderator), &request).await.unwrap();
    response.export.into_option().unwrap()
}

/// The exported archive, read straight from the private bucket.
async fn exported_archive(ctx: &Ctx, export_id: &str) -> (String, Vec<u8>) {
    let key: String = sqlx::query_scalar("SELECT object_key FROM emoji_exports WHERE id = $1::uuid")
        .bind(export_id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    let bytes = ctx.app.core.storage.get(Bucket::Private, &key).await.unwrap().to_vec();
    (key, bytes)
}

fn file_of(archive: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    archive.by_name(name).unwrap_or_else(|e| panic!("{name}: {e}")).read_to_end(&mut bytes).unwrap();
    bytes
}

#[tokio::test]
async fn the_catalogue_is_exported_as_a_pack_that_can_be_imported_again() {
    let ctx = start().await;
    ctx.app.core.storage.ensure_buckets().await.unwrap();
    let moderator = ctx.moderator().await;
    let t = unique("");
    let category = format!("えくすぽーと{t}");
    ctx.create_category(&moderator, &category).await;
    let (still, moving, lost) = (format!("a_still{t}"), format!("b_moving{t}"), format!("c_lost{t}"));
    let (still_png, moving_gif) = (png(48), gif(32));

    let request = AddEmojiRequest {
        category: category.clone(),
        aliases: strings(&["ねこ", "cat"]),
        license: "CC BY 4.0".to_owned(),
        sensitive: true,
        ..ctx.add_request(&moderator, &still).await
    };
    store_original(&ctx, &request.asset_id, &still_png, "image/png").await;
    let still_id = ctx.add_with(&moderator, request).await.emoji.id.clone();
    let request = AddEmojiRequest {
        category: category.clone(),
        local_only: true,
        asset_id: ctx.animation(&moderator).await,
        ..ctx.add_request(&moderator, &moving).await
    };
    store_original(&ctx, &request.asset_id, &moving_gif, "image/gif").await;
    ctx.add_with(&moderator, request).await;
    // An emoji whose original cannot be read is listed without a file.
    let request = AddEmojiRequest { category: category.clone(), ..ctx.add_request(&moderator, &lost).await };
    ctx.add_with(&moderator, request).await;
    let elsewhere = ctx.add(&moderator, &format!("d_elsewhere{t}")).await;

    let mut inbox = ctx.app.listen_user(moderator.id).await;
    let queued = start_export(&ctx, &moderator, Some(&category.to_uppercase())).await;
    assert_eq!(queued.status, ExportStatus::EXPORT_STATUS_QUEUED);
    assert!(queued.by_category && queued.download_url.is_empty());
    assert_eq!(queued.category, category);

    ctx.emoji.run_export(queued.id.parse().unwrap()).await.unwrap();
    let ready = get_export(&ctx, &moderator, &queued.id).await;
    assert_eq!(ready.status, ExportStatus::EXPORT_STATUS_READY);
    assert_eq!(ready.emoji_count, 3);
    assert!(ready.size_bytes > 0 && ready.completed_at.is_set() && ready.expires_at.is_set() && ready.error.is_empty());
    let (key, bytes) = exported_archive(&ctx, &queued.id).await;
    assert_eq!(bytes.len() as i64, ready.size_bytes);
    assert!(ready.download_url.contains(&key), "the link leads to the archive: {}", ready.download_url);
    let told = next_matching(&mut inbox, |frame| match emoji_event(frame)? {
        emoji_event::Event::ExportUpdated(export) if export.id == queued.id => Some(export.status),
        _ => None,
    })
    .await;
    assert_eq!(told, Some(ExportStatus::EXPORT_STATUS_READY.into()));

    // The archive is a Misskey pack: meta.json plus the pictures as uploaded.
    let mut archive = ZipArchive::new(Cursor::new(bytes.clone())).unwrap();
    let meta: serde_json::Value = serde_json::from_slice(&file_of(&mut archive, "meta.json")).unwrap();
    assert_eq!((meta["metaVersion"].as_i64(), meta["host"].as_str()), (Some(2), Some("localhost")));
    assert!(meta["exportedAt"].is_string());
    let records = meta["emojis"].as_array().unwrap();
    let names: Vec<&str> = records.iter().map(|record| record["emoji"]["name"].as_str().unwrap()).collect();
    assert_eq!(names, [still.as_str(), moving.as_str(), lost.as_str()], "ordered by name, this category only");
    let first = &records[0];
    assert_eq!(
        (first["fileName"].as_str(), first["downloaded"].as_bool()),
        (Some(format!("{still}.png").as_str()), Some(true))
    );
    let emoji = &first["emoji"];
    assert_eq!(emoji["id"], still_id);
    assert_eq!(
        (emoji["category"].as_str(), emoji["license"].as_str(), emoji["type"].as_str()),
        (Some(category.as_str()), Some("CC BY 4.0"), Some("image/png"))
    );
    assert_eq!(emoji["aliases"], serde_json::json!(["ねこ", "cat"]));
    assert_eq!((emoji["isSensitive"].as_bool(), emoji["localOnly"].as_bool()), (Some(true), Some(false)));
    assert!(emoji["host"].is_null() && emoji["uri"].is_null());
    assert_eq!(emoji["roleIdsThatCanBeUsedThisEmojiAsReaction"], serde_json::json!([]));
    assert!(emoji["publicUrl"].as_str().unwrap().starts_with("http") && emoji["updatedAt"].is_string());
    assert_eq!(file_of(&mut archive, &format!("{still}.png")), still_png, "the picture is the upload, byte for byte");
    assert_eq!(records[1]["fileName"], format!("{moving}.gif"));
    assert_eq!(records[1]["emoji"]["localOnly"], true);
    assert_eq!(file_of(&mut archive, &format!("{moving}.gif")), moving_gif);
    assert_eq!(records[2]["downloaded"], false);
    assert_eq!(archive.len(), 3, "meta.json and the two pictures that could be read");

    // The pack reads back in: the names are taken, so they are numbered.
    let pack = upload_archive(&ctx, &moderator, bytes).await;
    let import = start_import(&ctx, &moderator, &pack, ImportConflictPolicy::IMPORT_CONFLICT_POLICY_RENAME, "").await;
    run_import(&ctx, &import.id).await;
    let uploads = process_pictures(&ctx, &import.id).await;
    deliver_media_events(&ctx, &uploads).await;
    let done = get_import(&ctx, &moderator, &import.id).await;
    assert_eq!(done.import.status, ImportStatus::IMPORT_STATUS_COMPLETED);
    assert_eq!(
        (done.import.source_host.as_str(), done.import.imported_count, done.import.failed_count),
        ("localhost", 2, 1)
    );
    assert_eq!(entry(&done, &lost).reason, "NOT_DOWNLOADED");
    let copy = ctx.get(None, &format!("{still}_2")).await.unwrap();
    assert_eq!((copy.emoji.category.as_str(), copy.license.as_str()), (category.as_str(), "CC BY 4.0"));
    assert_eq!(copy.emoji.aliases, strings(&["ねこ", "cat"]));
    assert!(copy.emoji.sensitive && !copy.emoji.animated);
    let moving_copy = ctx.get(None, &format!("{moving}_2")).await.unwrap();
    assert!(moving_copy.emoji.animated && moving_copy.emoji.local_only);

    // The whole catalogue, and the emoji without a category.
    let everything = start_export(&ctx, &moderator, None).await;
    assert!(!everything.by_category);
    ctx.emoji.run_export(everything.id.parse().unwrap()).await.unwrap();
    assert!(get_export(&ctx, &moderator, &everything.id).await.emoji_count >= 6);
    let loose = start_export(&ctx, &moderator, Some("")).await;
    assert!(loose.by_category && loose.category.is_empty());
    ctx.emoji.run_export(loose.id.parse().unwrap()).await.unwrap();
    let (_, bytes) = exported_archive(&ctx, &loose.id).await;
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let meta: serde_json::Value = serde_json::from_slice(&file_of(&mut archive, "meta.json")).unwrap();
    let names: Vec<&str> =
        meta["emojis"].as_array().unwrap().iter().map(|record| record["emoji"]["name"].as_str().unwrap()).collect();
    assert!(names.contains(&elsewhere.emoji.name.as_str()) && !names.contains(&still.as_str()));

    let listed: ListExportsResponse =
        ctx.call("Admin", "ListExports", Some(&moderator), &ListExportsRequest::default()).await.unwrap();
    let ids: Vec<&str> = listed.exports.iter().map(|export| export.id.as_str()).collect();
    assert!(ids.contains(&queued.id.as_str()) && ids.contains(&everything.id.as_str()));
    let unknown = StartExportRequest { category: Some(unique("nocategory")), ..Default::default() };
    assert_eq!(ctx.reason("Admin", "StartExport", Some(&moderator), &unknown).await, "CATEGORY_NOT_FOUND");
    let missing = GetExportRequest { export_id: Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(ctx.reason("Admin", "GetExport", Some(&moderator), &missing).await, "EXPORT_NOT_FOUND");

    // After its retention the file is deleted and the export says so.
    sqlx::query("UPDATE emoji_exports SET expires_at = now() - interval '1 minute' WHERE id = $1::uuid")
        .bind(&queued.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.emoji.maintain().await.unwrap();
    let expired = get_export(&ctx, &moderator, &queued.id).await;
    assert_eq!(expired.status, ExportStatus::EXPORT_STATUS_EXPIRED);
    assert!(expired.download_url.is_empty());
    assert_eq!(ctx.app.core.storage.head(Bucket::Private, &key).await.unwrap(), None);
    assert_eq!(get_export(&ctx, &moderator, &everything.id).await.status, ExportStatus::EXPORT_STATUS_READY);
}
