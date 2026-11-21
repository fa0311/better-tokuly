//! A pack's way into the store as its creator sees it: submitting, the
//! editing rules of each status, trusted creators, official packs, and the
//! statistics of a published pack.
mod common;

use common::*;
use tokuly_proto::stamp::*;
use tokuly_stamp::{SendContext, record_send};

#[tokio::test]
async fn submitting_sends_the_pack_to_review_and_locks_it() {
    let ctx = start().await;
    let creator = ctx.app.user("creator").await;
    let id = id_of(&ctx.draft(&creator, "しんさ待ち", 0, &[]).await);
    let submit = SubmitPackRequest { pack_id: id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Studio", "SubmitPack", Some(&creator), &submit).await, "PACK_EMPTY");

    let pack = ctx.add_stamps(&creator, &id, "s", 2).await;
    let stamp_id = pack.stamps[0].stamp_ref.id.clone();
    let submitted = ctx.submit(&creator, &id).await;
    assert_eq!(submitted.summary.status, PackStatus::PACK_STATUS_IN_REVIEW);
    assert!(submitted.review.submitted_at.is_set());
    let events = ctx.app.events_by("stamp.pack.submitted", creator.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data["pack_id"], id);

    // Nothing changes while a moderator is looking.
    let locked = "PACK_LOCKED";
    assert_eq!(
        ctx.reason(
            "Studio",
            "UpdatePack",
            Some(&creator),
            &UpdatePackRequest { pack_id: id.clone(), name: Some("x".into()), ..Default::default() }
        )
        .await,
        locked
    );
    let more = AddStampsRequest {
        pack_id: id.clone(),
        stamps: vec![ctx.new_stamp(&creator, "x").await],
        ..Default::default()
    };
    assert_eq!(ctx.reason("Studio", "AddStamps", Some(&creator), &more).await, locked);
    assert_eq!(
        ctx.reason(
            "Studio",
            "UpdateStamp",
            Some(&creator),
            &UpdateStampRequest { stamp_id: stamp_id.clone(), name: Some("x".into()), ..Default::default() }
        )
        .await,
        locked
    );
    assert_eq!(
        ctx.reason(
            "Studio",
            "RemoveStamp",
            Some(&creator),
            &RemoveStampRequest { stamp_id: stamp_id.clone(), ..Default::default() }
        )
        .await,
        locked
    );
    let order: Vec<String> = pack.stamps.iter().map(|s| s.stamp_ref.id.clone()).collect();
    assert_eq!(
        ctx.reason(
            "Studio",
            "ReorderStamps",
            Some(&creator),
            &ReorderStampsRequest { pack_id: id.clone(), stamp_ids: order, ..Default::default() }
        )
        .await,
        locked
    );
    assert_eq!(ctx.reason("Studio", "SubmitPack", Some(&creator), &submit).await, "PACK_NOT_SUBMITTABLE");
    assert_eq!(
        ctx.reason("Studio", "SubmitPack", Some(&ctx.app.user("stranger").await), &submit).await,
        "PACK_NOT_FOUND"
    );

    // Withdrawing makes it a draft again.
    let unpublish = UnpublishPackRequest { pack_id: id.clone(), ..Default::default() };
    let withdrawn: UnpublishPackResponse =
        ctx.call("Studio", "UnpublishPack", Some(&creator), &unpublish).await.unwrap();
    assert_eq!(withdrawn.pack.summary.status, PackStatus::PACK_STATUS_DRAFT);
    assert_eq!(ctx.reason("Studio", "UnpublishPack", Some(&creator), &unpublish).await, "PACK_NOT_PUBLISHED");
    let _: UpdatePackResponse = ctx
        .call(
            "Studio",
            "UpdatePack",
            Some(&creator),
            &UpdatePackRequest { pack_id: id.clone(), name: Some("なおした".into()), ..Default::default() },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn published_packs_take_new_stamps_only_after_another_review() {
    let ctx = start().await;
    let (creator, fan) = (ctx.app.user("creator").await, ctx.app.user("fan").await);
    let pack = ctx.published(&creator, "こうかい中", 0, &[], 2).await;
    let id = id_of(&pack);
    assert_eq!(pack.summary.status, PackStatus::PACK_STATUS_PUBLISHED);
    assert!(pack.stamps.iter().all(|s| !s.pending_review));
    ctx.install(&fan, &id).await;

    let more = AddStampsRequest {
        pack_id: id.clone(),
        stamps: vec![ctx.new_stamp(&creator, "x").await],
        ..Default::default()
    };
    assert_eq!(ctx.reason("Studio", "AddStamps", Some(&creator), &more).await, "PACK_LOCKED");
    // Listing edits need no review and stay possible.
    let _: UpdatePackResponse = ctx
        .call(
            "Studio",
            "UpdatePack",
            Some(&creator),
            &UpdatePackRequest {
                pack_id: id.clone(),
                description: Some("あたらしい説明".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Unpublish, add a stamp: the fan keeps the pack but only sees what was reviewed.
    ctx.unpublish(&creator, &id).await;
    assert_eq!(ctx.app.events_by("stamp.pack.unpublished", creator.id).await.len(), 1);
    let edited = ctx.add_stamps(&creator, &id, "あたらしい", 1).await;
    assert_eq!(edited.stamps.len(), 3);
    assert!(edited.stamps[2].pending_review);
    let seen_by_fan = ctx.get_pack(Some(&fan), &id).await.unwrap();
    assert_eq!(seen_by_fan.stamps.len(), 2);
    assert_eq!(seen_by_fan.summary.stamp_count, 2);
    assert!(seen_by_fan.review.is_unset(), "review state is not for ordinary viewers");
    assert_eq!(ctx.get_pack(None, &id).await.unwrap_err().reason, "PACK_NOT_FOUND", "it left the store");

    // Submitting again goes back through review, because a stamp was added.
    assert_eq!(ctx.submit(&creator, &id).await.summary.status, PackStatus::PACK_STATUS_IN_REVIEW);
    let approved = ctx.approve(&id).await;
    assert_eq!(approved.stamps.len(), 3);
    assert_eq!(ctx.get_pack(Some(&fan), &id).await.unwrap().stamps.len(), 3);

    // Unpublishing and publishing again without changes needs no second look.
    ctx.unpublish(&creator, &id).await;
    let draft = ctx.get_pack(Some(&creator), &id).await.unwrap();
    assert!(!draft.review.needs_review);
    assert_eq!(ctx.submit(&creator, &id).await.summary.status, PackStatus::PACK_STATUS_PUBLISHED);
    assert_eq!(
        ctx.search_changes(&id).await,
        vec![true, true, false, true, false, true],
        "indexed while in the store, removed while not"
    );
}

#[tokio::test]
async fn trusted_creators_publish_without_review_and_edit_live() {
    let ctx = start().await;
    let (creator, fan, moderator) = (ctx.app.user("trusted").await, ctx.app.user("fan").await, ctx.moderator().await);
    let trust = SetCreatorTrustedRequest { user_id: creator.id.to_string(), trusted: true, ..Default::default() };
    let _: SetCreatorTrustedResponse = ctx.call("Review", "SetCreatorTrusted", Some(&moderator), &trust).await.unwrap();

    let id = id_of(&ctx.draft(&creator, "しんらい", 0, &[]).await);
    let draft = ctx.add_stamps(&creator, &id, "s", 1).await;
    assert!(!draft.review.needs_review, "the studio can tell the creator it will go live at once");
    let published = ctx.submit(&creator, &id).await;
    assert_eq!(published.summary.status, PackStatus::PACK_STATUS_PUBLISHED);
    assert!(published.summary.published_at.is_set());
    assert!(
        ctx.notifications(&creator, "stamp.pack.approved").await.is_empty(),
        "nobody reviewed it, so there is nothing to announce"
    );
    let published_events = ctx.app.events_by("stamp.pack.published", creator.id).await;
    assert_eq!(published_events.len(), 1);

    // Stamps added to the live pack are live at once.
    let live = ctx.add_stamps(&creator, &id, "追加", 1).await;
    assert!(live.stamps.iter().all(|s| !s.pending_review));
    assert_eq!(ctx.get_pack(Some(&fan), &id).await.unwrap().stamps.len(), 2);

    let profile: GetCreatorProfileResponse =
        ctx.call("Studio", "GetCreatorProfile", Some(&creator), &GetCreatorProfileRequest::default()).await.unwrap();
    assert!(profile.profile.trusted);
    assert_eq!((profile.profile.pack_count, profile.profile.published_pack_count), (1, 1));
}

#[tokio::test]
async fn admins_make_official_packs_that_publish_without_review() {
    let ctx = start().await;
    let (admin, moderator, user) =
        (ctx.app.user_with_roles("admin", &["admin"]).await, ctx.moderator().await, ctx.app.user("user").await);
    let tag = unique("official");
    let request = CreatePackRequest {
        name: "こうしきパック".into(),
        tags: vec![tag.clone()],
        official: true,
        ..Default::default()
    };
    assert_eq!(ctx.reason("Studio", "CreatePack", Some(&moderator), &request).await, "FORBIDDEN");
    let created: CreatePackResponse = ctx.call("Studio", "CreatePack", Some(&admin), &request).await.unwrap();
    let id = created.pack.summary.id.clone();
    assert!(created.pack.summary.official && created.pack.summary.creator.is_unset());
    assert!(!created.pack.summary.viewer.installed, "official drafts are not put in the admin's own picker");

    // Any admin manages it; moderators can look but not edit.
    let other_admin = ctx.app.user_with_roles("admin2", &["admin"]).await;
    ctx.add_stamps(&other_admin, &id, "公式", 2).await;
    assert!(ctx.get_pack(Some(&moderator), &id).await.is_ok());
    let edit = UpdatePackRequest { pack_id: id.clone(), name: Some("x".into()), ..Default::default() };
    assert_eq!(ctx.reason("Studio", "UpdatePack", Some(&moderator), &edit).await, "PACK_NOT_FOUND");
    assert_eq!(ctx.get_pack(Some(&user), &id).await.unwrap_err().reason, "PACK_NOT_FOUND");

    let listing = ListMyPacksRequest { official: true, ..Default::default() };
    assert_eq!(ctx.reason("Studio", "ListMyPacks", Some(&user), &listing).await, "FORBIDDEN");
    let official: ListMyPacksResponse = ctx.call("Studio", "ListMyPacks", Some(&admin), &listing).await.unwrap();
    assert!(official.packs.iter().any(|p| p.id == id));

    assert_eq!(ctx.submit(&admin, &id).await.summary.status, PackStatus::PACK_STATUS_PUBLISHED);
    let shelf = ListStorePacksRequest {
        section: StoreSection::STORE_SECTION_OFFICIAL.into(),
        tag: tag.clone(),
        ..Default::default()
    };
    let listed: ListStorePacksResponse = ctx.call("Store", "ListStorePacks", Some(&user), &shelf).await.unwrap();
    assert_eq!(pack_names(&listed.packs), vec!["こうしきパック"]);
    // Free and official: usable by anyone without installing.
    let pack = ctx.get_pack(Some(&user), &id).await.unwrap();
    assert!(tokuly_stamp::ensure_usable(&ctx.app.core, user.id, stamp_uuid(&pack, 0)).await.is_ok());
}

#[tokio::test]
async fn pack_stats_count_sends_installs_and_sales() {
    let ctx = start().await;
    let (creator, fan, buyer) = (ctx.app.user("creator").await, ctx.app.user("fan").await, ctx.app.user("buyer").await);
    let pack = ctx.published(&creator, "にんき", 200, &[], 3).await;
    let id = id_of(&pack);
    ctx.grant_coins(&buyer, 200).await;
    ctx.grant_coins(&fan, 200).await;
    for user in [&buyer, &fan] {
        ctx.buy(user, &id, 200).await;
    }
    let (first, second) = (stamp_uuid(&pack, 0), stamp_uuid(&pack, 1));
    for (user, stamp, context) in [
        (&fan, first, SendContext::Chat),
        (&fan, first, SendContext::Chat),
        (&buyer, first, SendContext::LiveChat),
        (&buyer, second, SendContext::Comment),
        (&fan, second, SendContext::Map),
    ] {
        record_send(&ctx.app.core, user.id, stamp, context).await.unwrap();
    }
    // The creator's own sends are logged but do not move the numbers.
    record_send(&ctx.app.core, creator.id, first, SendContext::Note).await.unwrap();

    let stats: GetPackStatsResponse = ctx
        .call(
            "Studio",
            "GetPackStats",
            Some(&creator),
            &GetPackStatsRequest { pack_id: id.clone(), days: 7, ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!((stats.install_count, stats.sale_count, stats.send_count), (2, 2, 5));
    assert_eq!(stats.earnings, 280, "two sales of 200 coins minus the 30% fee");
    assert_eq!(stats.daily.len(), 7);
    let today = stats.daily.last().unwrap();
    assert_eq!((today.sends, today.installs, today.sales, today.earnings), (5, 2, 2, 280));
    assert!(
        stats.daily[..6].iter().all(|day| day.sends == 0 && day.installs == 0),
        "days without activity are zero-filled"
    );
    assert!(stats.daily.windows(2).all(|pair| pair[0].date < pair[1].date), "oldest first");
    let by_context: Vec<(UsageContext, i64)> =
        stats.contexts.iter().map(|c| (c.context.as_known().unwrap(), c.sends)).collect();
    assert_eq!(
        by_context,
        vec![
            (UsageContext::USAGE_CONTEXT_CHAT, 2),
            (UsageContext::USAGE_CONTEXT_LIVE_CHAT, 1),
            (UsageContext::USAGE_CONTEXT_COMMENT, 1),
            (UsageContext::USAGE_CONTEXT_NOTE, 0),
            (UsageContext::USAGE_CONTEXT_MAP, 1),
        ]
    );
    let top: Vec<(String, i64)> = stats.top_stamps.iter().map(|t| (t.stamp.stamp_ref.name.clone(), t.sends)).collect();
    assert_eq!(top, vec![("スタンプ1".to_owned(), 3), ("スタンプ2".to_owned(), 2)]);

    let default_window: GetPackStatsResponse = ctx
        .call(
            "Studio",
            "GetPackStats",
            Some(&creator),
            &GetPackStatsRequest { pack_id: id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(default_window.daily.len(), 30);
    assert_eq!(
        ctx.reason(
            "Studio",
            "GetPackStats",
            Some(&fan),
            &GetPackStatsRequest { pack_id: id.clone(), ..Default::default() }
        )
        .await,
        "PACK_NOT_FOUND"
    );

    let profile: GetCreatorProfileResponse =
        ctx.call("Studio", "GetCreatorProfile", Some(&creator), &GetCreatorProfileRequest::default()).await.unwrap();
    let profile = profile.profile.into_option().unwrap();
    assert_eq!((profile.install_count, profile.sale_count, profile.send_count, profile.earnings), (2, 2, 5, 280));
    assert_eq!(
        (profile.fee_percent, profile.max_stamps_per_pack, profile.min_price, profile.max_price),
        (30, 120, 10, 10_000)
    );
    assert!(!profile.trusted);
    // Someone who never made a pack has an empty profile, not an error.
    let newcomer: GetCreatorProfileResponse =
        ctx.call("Studio", "GetCreatorProfile", Some(&fan), &GetCreatorProfileRequest::default()).await.unwrap();
    assert_eq!((newcomer.profile.pack_count, newcomer.profile.earnings), (0, 0));
}
