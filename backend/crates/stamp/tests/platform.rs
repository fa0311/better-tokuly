//! The stamp domain as the rest of the platform meets it: the cross-domain
//! API, the events it consumes, the workers, the official packs, and what it
//! plugs into the generic domains (subjects, data export).
mod common;

use common::*;
use tokuly_proto::{common::SubjectType, stamp::*};
use tokuly_stamp::{SendContext, ensure_usable, record_send, stamp_refs, stats};
use uuid::Uuid;

/// Polls `check` until it holds, for at most 30 seconds.
async fn eventually<F, Fut>(what: &str, check: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..300 {
        if check().await {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("timed out waiting until {what}");
}

#[tokio::test]
async fn private_rpcs_require_sign_in_and_review_requires_the_moderator_role() {
    let ctx = start().await;
    let user = ctx.app.user("user").await;
    let empty = serde_json::json!({});
    let library = [
        "GetLibrary",
        "ListInstalledPacks",
        "InstallPack",
        "UninstallPack",
        "ReorderInstalledPacks",
        "PurchasePack",
        "ListPurchasedPacks",
        "ListFavoriteStamps",
        "FavoriteStamp",
        "UnfavoriteStamp",
        "ListRecentStamps",
        "ClearRecentStamps",
        "SaveStamp",
        "CreatePersonalStamp",
        "ListPersonalStamps",
        "UpdatePersonalStamp",
        "DeletePersonalStamp",
        "ReorderPersonalStamps",
        "SearchLibraryStamps",
        "ListUsageLog",
    ];
    let studio = [
        "GetCreatorProfile",
        "ListMyPacks",
        "CreatePack",
        "UpdatePack",
        "DeletePack",
        "AddStamps",
        "UpdateStamp",
        "RemoveStamp",
        "ReorderStamps",
        "SubmitPack",
        "UnpublishPack",
        "GetPackStats",
    ];
    let review = ["ListReviewQueue", "ApprovePack", "RejectPack", "SetCreatorTrusted", "SetPackFeatured"];

    // The caller is checked before anything in the request is looked at.
    for (service, methods) in [("Library", &library[..]), ("Studio", &studio[..]), ("Review", &review[..])] {
        for method in methods {
            assert_eq!(ctx.reason(service, method, None, &empty).await, "SIGN_IN_REQUIRED", "{service}/{method}");
        }
    }
    for method in review {
        assert_eq!(ctx.reason("Review", method, Some(&user), &empty).await, "FORBIDDEN", "Review/{method}");
    }
    // The store is open to everyone.
    let home: GetStoreHomeResponse = ctx.call("Store", "GetStoreHome", None, &empty).await.unwrap();
    assert!(home.featured.len() <= 12);
}

#[tokio::test]
async fn other_domains_resolve_check_and_record_stamps() {
    let ctx = start().await;
    let (creator, user) = (ctx.app.user("creator").await, ctx.app.user("user").await);
    let core = &ctx.app.core;
    let free = ctx.published(&creator, "むりょう", 0, &[], 1).await;
    let paid = ctx.published(&creator, "ゆうりょう", 100, &[], 1).await;
    let draft_id = id_of(&ctx.draft(&creator, "したがき", 0, &[]).await);
    let draft = ctx.add_stamps(&creator, &draft_id, "みかんせい", 1).await;
    let personal = ctx.personal(&user, "じぶんの").await;
    let (free_stamp, paid_stamp, draft_stamp) = (stamp_uuid(&free, 0), stamp_uuid(&paid, 0), stamp_uuid(&draft, 0));
    let personal_stamp: Uuid = personal.stamp_ref.id.parse().unwrap();
    let unknown = Uuid::now_v7();

    // ensure_usable: free, own, personal — yes; paid without buying, unreviewed, unknown — no.
    let usable = ensure_usable(core, user.id, free_stamp).await.unwrap();
    assert_eq!(
        (usable.id.clone(), usable.pack_id.clone(), usable.name.as_str()),
        (free_stamp.to_string(), id_of(&free), "スタンプ1")
    );
    assert!(usable.image_url.ends_with("/512.webp") && usable.width == 512);
    assert!(ensure_usable(core, user.id, personal_stamp).await.is_ok());
    assert!(ensure_usable(core, creator.id, paid_stamp).await.is_ok(), "creators use their own packs");
    assert!(ensure_usable(core, creator.id, draft_stamp).await.is_ok(), "even before they are published");
    for (who, stamp, why) in [
        (&user, paid_stamp, "a paid pack that was not bought"),
        (&user, draft_stamp, "someone else's unpublished stamp"),
        (&creator, personal_stamp, "someone else's personal stamp"),
        (&user, unknown, "a stamp that does not exist"),
    ] {
        let error = ensure_usable(core, who.id, stamp).await.unwrap_err();
        assert_eq!(error.reason, "STAMP_NOT_USABLE", "{why}");
    }

    // stamp_refs: whoever sees the message sees the stamp, whatever its pack's state.
    let refs = stamp_refs(core, &[free_stamp, paid_stamp, draft_stamp, personal_stamp, unknown]).await.unwrap();
    assert_eq!(refs.len(), 4);
    assert!(!refs.contains_key(&unknown));
    assert_eq!(refs[&personal_stamp].pack_id, "");
    assert_eq!(refs[&draft_stamp].name, "みかんせい1");
    assert!(stamp_refs(core, &[]).await.unwrap().is_empty());

    // record_send: the log, the counters and the day's numbers.
    record_send(core, user.id, free_stamp, SendContext::Chat).await.unwrap();
    record_send(core, user.id, free_stamp, SendContext::Note).await.unwrap();
    record_send(core, user.id, personal_stamp, SendContext::Map).await.unwrap();
    let sends: i64 = sqlx::query_scalar("SELECT send_count FROM stamps WHERE id = $1")
        .bind(free_stamp)
        .fetch_one(&core.db)
        .await
        .unwrap();
    assert_eq!(sends, 2);
    let (chat, note, total): (i64, i64, i64) =
        sqlx::query_as("SELECT chat_sends, note_sends, sends FROM stamp_pack_daily WHERE pack_id = $1")
            .bind(Uuid::parse_str(&id_of(&free)).unwrap())
            .fetch_one(&core.db)
            .await
            .unwrap();
    assert_eq!((chat, note, total), (1, 1, 2));
    let logged: Vec<(Uuid, i32)> =
        sqlx::query_as("SELECT stamp_id, context FROM stamp_usage WHERE user_id = $1 ORDER BY id")
            .bind(user.id)
            .fetch_all(&core.db)
            .await
            .unwrap();
    assert_eq!(logged, vec![(free_stamp, 1), (free_stamp, 4), (personal_stamp, 5)]);
    let use_count: i32 = sqlx::query_scalar("SELECT use_count FROM stamp_recents WHERE user_id = $1 AND stamp_id = $2")
        .bind(user.id)
        .bind(free_stamp)
        .fetch_one(&core.db)
        .await
        .unwrap();
    assert_eq!(use_count, 2);
}

#[tokio::test]
async fn concurrent_sends_are_all_counted() {
    let ctx = start().await;
    let creator = ctx.app.user("creator").await;
    let core = &ctx.app.core;
    let pack = ctx.published(&creator, "いっせいに", 0, &[], 3).await;
    let mut senders = Vec::new();
    for n in 0..6 {
        senders.push(ctx.app.user(&format!("sender{n}")).await);
    }

    // A busy live chat: six people sending the pack's three stamps at once, five rounds each.
    let contexts = [SendContext::LiveChat, SendContext::Chat, SendContext::Comment];
    let sends =
        senders.iter().enumerate().flat_map(|(n, sender)| (0..5).map(move |round| (sender.id, (n + round) % 3))).map(
            |(sender_id, which)| {
                let pack = &pack;
                async move { record_send(core, sender_id, stamp_uuid(pack, which), contexts[which]).await }
            },
        );
    let results = futures::future::join_all(sends).await;
    assert!(results.iter().all(|result| result.is_ok()), "no send is lost to a deadlock or a conflict");

    let per_stamp: Vec<i64> = sqlx::query_scalar("SELECT send_count FROM stamps WHERE pack_id = $1 ORDER BY position")
        .bind(Uuid::parse_str(&id_of(&pack)).unwrap())
        .fetch_all(&core.db)
        .await
        .unwrap();
    assert_eq!(per_stamp, vec![10, 10, 10]);
    let (live, chat, comment, total): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT live_chat_sends, chat_sends, comment_sends, sends FROM stamp_pack_daily WHERE pack_id = $1",
    )
    .bind(Uuid::parse_str(&id_of(&pack)).unwrap())
    .fetch_one(&core.db)
    .await
    .unwrap();
    assert_eq!((live, chat, comment, total), (10, 10, 10, 30));
    let logged: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM stamp_usage u JOIN stamps s ON s.id = u.stamp_id WHERE s.pack_id = $1",
    )
    .bind(Uuid::parse_str(&id_of(&pack)).unwrap())
    .fetch_one(&core.db)
    .await
    .unwrap();
    assert_eq!(logged, 30);
}

#[tokio::test]
async fn deleting_an_account_erases_its_stamps_packs_and_library() {
    let ctx = start().await;
    let (leaver, friend, buyer) =
        (ctx.app.user("leaver").await, ctx.app.user("friend").await, ctx.app.user("buyer").await);
    let core = &ctx.app.core;
    let theirs = ctx.published(&leaver, "やめる人の", 0, &[], 2).await;
    let draft = id_of(&ctx.draft(&leaver, "したがき", 0, &[]).await);
    let personal = ctx.personal(&leaver, "じぶんの").await;
    let friends = ctx.published(&friend, "ともだちの", 0, &[], 1).await;
    // Each holds something of the other.
    ctx.install(&friend, &id_of(&theirs)).await;
    ctx.install(&leaver, &id_of(&friends)).await;
    record_send(core, friend.id, stamp_uuid(&theirs, 0), SendContext::Chat).await.unwrap();
    record_send(core, leaver.id, stamp_uuid(&friends, 0), SendContext::Chat).await.unwrap();
    ctx.favorite(&leaver, &friends.stamps[0].stamp_ref.id).await;
    let saved = ctx.save(&friend, &personal.stamp_ref.id).await;
    // A pack somebody bought, back in review with one more stamp nobody else has seen.
    let sold = ctx.published(&leaver, "うれたパック", 100, &[], 2).await;
    let sold_id = id_of(&sold);
    ctx.grant_coins(&buyer, 100).await;
    ctx.buy(&buyer, &sold_id, 100).await;
    ctx.unpublish(&leaver, &sold_id).await;
    let unreviewed = ctx.add_stamps(&leaver, &sold_id, "みかんせい", 1).await.stamps[2].clone();
    ctx.submit(&leaver, &sold_id).await;

    let deleted = event("identity.user.deleted", serde_json::json!({ "user_id": leaver.id }));
    ctx.stamp.handle_event(deleted.clone()).await.unwrap();
    ctx.stamp.handle_event(deleted).await.unwrap();

    // Their packs and stamps are gone for everyone, and the uploads are let go…
    assert_eq!(ctx.get_pack(None, &id_of(&theirs)).await.unwrap_err().reason, "PACK_NOT_FOUND");
    assert_eq!(ctx.get_pack(Some(&leaver), &draft).await.unwrap_err().reason, "PACK_NOT_FOUND");
    let gone = [stamp_uuid(&theirs, 0), stamp_uuid(&theirs, 1), personal.stamp_ref.id.parse().unwrap()];
    assert!(stamp_refs(core, &gone).await.unwrap().is_empty());
    assert_eq!(ctx.search_changes(&id_of(&theirs)).await, vec![true, false]);
    for stamp in &theirs.stamps {
        assert!(ctx.holders(asset_of(stamp)).await.is_empty(), "nothing holds the upload of a deleted stamp");
    }
    let friends_library: GetLibraryResponse =
        ctx.call("Library", "GetLibrary", Some(&friend), &GetLibraryRequest::default()).await.unwrap();
    assert_eq!(pack_names(&friends_library.installed), vec!["ともだちの"], "only the friend's own pack is left");
    assert!(friends_library.recents.is_empty());

    // …except what other people hold in their own right. The copy the friend saved is the friend's:
    assert_eq!(stamp_names(&friends_library.personal), vec!["じぶんの"]);
    let copy = saved.stamp.into_option().unwrap();
    assert!(ensure_usable(core, friend.id, copy.stamp_ref.id.parse().unwrap()).await.is_ok());
    assert_eq!(ctx.holders(asset_of(&personal)).await, vec![hold_of(&copy)], "the copy alone keeps the upload now");

    // and the pack the buyer paid for stays theirs, as it was reviewed, out of the store and of review.
    let kept = ctx.get_pack(Some(&buyer), &sold_id).await.unwrap();
    assert_eq!(kept.summary.status, PackStatus::PACK_STATUS_DRAFT);
    assert_eq!(stamp_names(&kept.stamps), vec!["スタンプ1", "スタンプ2"]);
    assert!(kept.summary.viewer.owned && kept.summary.viewer.installed);
    assert!(ensure_usable(core, buyer.id, stamp_uuid(&sold, 0)).await.is_ok());
    assert_eq!(ctx.get_pack(None, &sold_id).await.unwrap_err().reason, "PACK_NOT_FOUND");
    assert_eq!(ctx.search_changes(&sold_id).await, vec![true, false]);
    assert_eq!(ctx.holders(asset_of(&sold.stamps[0])).await, vec![hold_of(&sold.stamps[0])]);
    assert!(stamp_refs(core, &[unreviewed.stamp_ref.id.parse().unwrap()]).await.unwrap().is_empty());
    assert!(ctx.holders(asset_of(&unreviewed)).await.is_empty(), "the stamp only its creator could see went with them");

    // Nothing of their own library or history remains.
    for table in [
        "stamp_pack_installs",
        "stamp_favorites",
        "stamp_recents",
        "stamp_usage",
        "stamp_creators",
        "stamp_pack_purchases",
    ] {
        let rows: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table} WHERE user_id = $1")))
                .bind(leaver.id)
                .fetch_one(&core.db)
                .await
                .unwrap();
        assert_eq!(rows, 0, "{table}");
    }
    assert_eq!(
        ctx.get_pack(Some(&friend), &id_of(&friends)).await.unwrap().summary.install_count,
        0,
        "their install no longer counts"
    );
}

#[tokio::test]
async fn workers_consume_events_from_the_stream_and_seed_at_startup() {
    let ctx = start_with(tokuly_stamp::StampConfig { seed_official: true, ..test_config() }).await;
    let core = &ctx.app.core;
    core.storage.ensure_buckets().await.unwrap();
    let leaver = ctx.app.user("leaver").await;
    ctx.personal(&leaver, "じぶんの").await;
    ctx.stamp.spawn_workers();

    // Published the way the outbox relay does it, the event reaches the durable consumer.
    let deleted = event("identity.user.deleted", serde_json::json!({ "user_id": leaver.id }));
    core.bus
        .publish_durable(
            format!("evt.{}", deleted.topic),
            serde_json::to_vec(&deleted).unwrap(),
            Some(&deleted.id.to_string()),
        )
        .await
        .unwrap();
    eventually("the deleted user's stamps are gone", || async {
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM stamps WHERE owner_id = $1")
            .bind(leaver.id)
            .fetch_one(&core.db)
            .await
            .unwrap();
        left == 0
    })
    .await;

    eventually("the official packs are published", || async {
        let published: i64 =
            sqlx::query_scalar("SELECT count(*) FROM stamp_packs WHERE official_key IS NOT NULL AND status = 3")
                .fetch_one(&core.db)
                .await
                .unwrap();
        published == 2
    })
    .await;
    core.shutdown.cancel();
}

#[tokio::test]
async fn the_official_packs_are_seeded_once() {
    let ctx = start().await;
    let core = &ctx.app.core;
    core.storage.ensure_buckets().await.unwrap();
    ctx.stamp.seed_official_packs().await.unwrap();
    ctx.stamp.seed_official_packs().await.unwrap();

    let seeded: Vec<(Uuid, String, i32)> = sqlx::query_as(
        "SELECT id, name, stamp_count FROM stamp_packs WHERE official_key IS NOT NULL ORDER BY official_key",
    )
    .fetch_all(&core.db)
    .await
    .unwrap();
    assert_eq!(
        seeded.iter().map(|(_, name, count)| (name.as_str(), *count)).collect::<Vec<_>>(),
        vec![("まるのきもち", 8), ("にゃんこのきもち", 8)]
    );

    let user = ctx.app.user("user").await;
    for (pack_id, _, _) in &seeded {
        let pack = ctx.get_pack(Some(&user), &pack_id.to_string()).await.unwrap();
        let summary = &pack.summary;
        assert_eq!(summary.status, PackStatus::PACK_STATUS_PUBLISHED);
        assert!(summary.official && summary.featured && summary.creator.is_unset() && summary.price == 0);
        assert!(summary.tags.contains(&"公式".to_owned()));
        assert_eq!(pack.stamps.len(), 8);
        for stamp in &pack.stamps {
            assert!(stamp.usable && !stamp.pending_review, "official stamps are for everyone, right away");
            assert!(!stamp.stamp_ref.name.is_empty() && !stamp.emoji.is_empty() && !stamp.keywords.is_empty());
            assert_eq!((stamp.stamp_ref.width, stamp.stamp_ref.height), (512, 512));
            assert_eq!(stamp.image.variants.len(), 1);
            assert!(!stamp.image.thumbhash.is_empty() && stamp.image.dominant_color.starts_with('#'));
        }
        // The pictures really are in the media bucket.
        let media_base = core.storage.media_url("");
        for url in [&pack.stamps[0].stamp_ref.image_url, &pack.stamps[7].image.variants[0].url] {
            let key = url.strip_prefix(media_base.as_str()).expect("served from the media bucket");
            let size = core.storage.head(tokuly_core::storage::Bucket::Media, key).await.unwrap();
            assert!(size.is_some_and(|bytes| bytes > 500), "{key} is stored");
        }
        assert!(ensure_usable(core, user.id, stamp_uuid(&pack, 0)).await.is_ok());
    }
    // Seeding published them like any other pack: in the store and in search.
    let shelf = ListStorePacksRequest {
        section: StoreSection::STORE_SECTION_OFFICIAL.into(),
        tag: "公式".into(),
        ..Default::default()
    };
    let official: ListStorePacksResponse = ctx.call("Store", "ListStorePacks", None, &shelf).await.unwrap();
    assert!(official.packs.len() >= 2);
    assert_eq!(ctx.search_changes(&seeded[0].0.to_string()).await, vec![true]);
}

#[tokio::test]
async fn packs_and_stamps_are_subjects_for_likes_comments_and_reports() {
    let ctx = start().await;
    let (creator, user) = (ctx.app.user("creator").await, ctx.app.user("user").await);
    let subjects = &ctx.app.core.subjects;
    let pack = ctx.published(&creator, "いいね", 0, &[], 1).await;
    let id = id_of(&pack);
    let pack_uuid: Uuid = id.parse().unwrap();

    let info =
        subjects.resolve(SubjectType::SUBJECT_TYPE_STAMP_PACK, &pack.summary.short_id, Some(user.id)).await.unwrap();
    assert_eq!((info.id, info.owner_id, info.title.as_str()), (pack_uuid, creator.id, "いいね"));
    assert_eq!(info.path, format!("/stamps/{}", pack.summary.short_id));
    assert!(info.interactions_enabled && info.image_url.is_some());
    assert!(
        subjects.resolve(SubjectType::SUBJECT_TYPE_STAMP_PACK, &id, None).await.is_ok(),
        "published packs are public"
    );

    subjects.counter_changed(SubjectType::SUBJECT_TYPE_STAMP_PACK, pack_uuid, "like_count", 2).await.unwrap();
    subjects.counter_changed(SubjectType::SUBJECT_TYPE_STAMP_PACK, pack_uuid, "comment_count", 1).await.unwrap();
    subjects.counter_changed(SubjectType::SUBJECT_TYPE_STAMP_PACK, pack_uuid, "like_count", -5).await.unwrap();
    subjects.counter_changed(SubjectType::SUBJECT_TYPE_STAMP_PACK, pack_uuid, "share_count", 1).await.unwrap();
    let seen = ctx.get_pack(None, &id).await.unwrap();
    assert_eq!(
        (seen.summary.like_count, seen.summary.comment_count),
        (0, 1),
        "counters never go negative; unknown ones are ignored"
    );

    // Drafts are not subjects for other people; malformed IDs are simply not found.
    let draft = id_of(&ctx.draft(&creator, "したがき", 0, &[]).await);
    assert_eq!(
        subjects.resolve(SubjectType::SUBJECT_TYPE_STAMP_PACK, &draft, Some(user.id)).await.unwrap_err().reason,
        "SUBJECT_NOT_FOUND"
    );
    assert!(
        !subjects
            .resolve(SubjectType::SUBJECT_TYPE_STAMP_PACK, &draft, Some(creator.id))
            .await
            .unwrap()
            .interactions_enabled
    );
    assert_eq!(
        subjects.resolve(SubjectType::SUBJECT_TYPE_STAMP_PACK, "?!", None).await.unwrap_err().reason,
        "SUBJECT_NOT_FOUND"
    );

    // Single stamps can be reported: they resolve to whoever answers for them.
    let stamp =
        subjects.resolve(SubjectType::SUBJECT_TYPE_STAMP, &pack.stamps[0].stamp_ref.id, Some(user.id)).await.unwrap();
    assert_eq!((stamp.owner_id, stamp.title.as_str(), stamp.interactions_enabled), (creator.id, "スタンプ1", false));
    assert_eq!(stamp.path, format!("/stamps/{}", pack.summary.short_id));
    let personal = ctx.personal(&user, "").await;
    let personal_info =
        subjects.resolve(SubjectType::SUBJECT_TYPE_STAMP, &personal.stamp_ref.id, Some(creator.id)).await.unwrap();
    assert_eq!(
        (personal_info.owner_id, personal_info.title.as_str(), personal_info.path.as_str()),
        (user.id, "スタンプ", "/stamps")
    );
    assert!(subjects.resolve(SubjectType::SUBJECT_TYPE_STAMP, "not-a-uuid", None).await.is_err());
}

#[tokio::test]
async fn the_data_export_contains_the_users_stamp_data() {
    let ctx = start().await;
    let (user, other) = (ctx.app.user("user").await, ctx.app.user("other").await);
    ctx.published(&user, "わたしの", 0, &[], 2).await;
    let theirs = ctx.published(&other, "ほかの", 0, &[], 1).await;
    ctx.install(&user, &id_of(&theirs)).await;
    ctx.personal(&user, "じぶんの").await;
    record_send(&ctx.app.core, user.id, stamp_uuid(&theirs, 0), SendContext::Comment).await.unwrap();

    let exporter = ctx
        .app
        .core
        .exporters
        .all()
        .into_iter()
        .find(|e| e.name() == "stamp")
        .expect("the stamp exporter is registered");
    let export = exporter.export(user.id).await.unwrap();
    assert_eq!(export["packs"].as_array().unwrap().len(), 1);
    assert_eq!(export["packs"][0]["name"], "わたしの");
    assert_eq!(export["packs"][0]["stamps"].as_array().unwrap().len(), 2);
    assert_eq!(export["personal_stamps"][0]["name"], "じぶんの");
    let installed: Vec<&str> =
        export["installed_packs"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(installed, vec!["ほかの", "わたしの"]);
    assert_eq!(export["usage_log"].as_array().unwrap().len(), 1);
    assert_eq!(export["usage_log"][0]["context"], 3);
    assert!(
        export["purchases"].as_array().unwrap().is_empty() && export["favorite_stamps"].as_array().unwrap().is_empty()
    );
}

#[tokio::test]
async fn pruning_drops_old_usage_rows_and_abandoned_purchase_intents() {
    let ctx = start().await;
    let (creator, user) = (ctx.app.user("creator").await, ctx.app.user("user").await);
    let core = &ctx.app.core;
    let pack = ctx.published(&creator, "ふるいログ", 100, &[], 1).await;
    let pack_uuid: Uuid = id_of(&pack).parse().unwrap();
    record_send(core, user.id, stamp_uuid(&pack, 0), SendContext::Chat).await.unwrap();
    record_send(core, user.id, stamp_uuid(&pack, 0), SendContext::Chat).await.unwrap();
    let oldest_row = "UPDATE stamp_usage SET created_at = now() - interval '200 days'
                      WHERE id = (SELECT id FROM stamp_usage WHERE user_id = $1 ORDER BY id LIMIT 1)";
    sqlx::query(oldest_row).bind(user.id).execute(&core.db).await.unwrap();
    // A purchase abandoned two days ago whose payment never went through.
    sqlx::query(
        "INSERT INTO stamp_purchase_intents (user_id, pack_id, price, payment_key, created_at)
         VALUES ($1, $2, 100, 'stamp-pack:abandoned:' || $1::text, now() - interval '2 days')",
    )
    .bind(user.id)
    .bind(pack_uuid)
    .execute(&core.db)
    .await
    .unwrap();

    assert!(stats::prune(core, 180).await.unwrap() >= 1);
    // It is settled, not swept: it was never paid, so it is over.
    assert!(ctx.stamp.settle_unfinished_purchases().await.unwrap() >= 1);
    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM stamp_usage WHERE user_id = $1")
        .bind(user.id)
        .fetch_one(&core.db)
        .await
        .unwrap();
    assert_eq!(remaining, 1, "only the row older than the retention period went");
    let intents: i64 = sqlx::query_scalar("SELECT count(*) FROM stamp_purchase_intents WHERE user_id = $1")
        .bind(user.id)
        .fetch_one(&core.db)
        .await
        .unwrap();
    assert_eq!(intents, 0);
    // The long-term numbers do not depend on the log.
    let stats: GetPackStatsResponse = ctx
        .call(
            "Studio",
            "GetPackStats",
            Some(&creator),
            &GetPackStatsRequest { pack_id: id_of(&pack), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(stats.send_count, 2);
    // A settled intent no longer protects the pack from deletion.
    ctx.delete_pack(&creator, &id_of(&pack)).await;
}
