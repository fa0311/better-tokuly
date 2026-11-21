//! The creator studio: making packs and editing them and their stamps.
mod common;

use common::*;
use tokuly_proto::stamp::*;
use tokuly_stamp::StampConfig;

#[tokio::test]
async fn a_new_pack_is_a_private_draft_in_its_creators_picker() {
    let ctx = start().await;
    let (creator, stranger) = (ctx.app.user("creator").await, ctx.app.user("stranger").await);

    let pack = ctx.draft(&creator, "はじめてのスタンプ", 0, &["Cute", "#ねこ"]).await;
    let summary = &pack.summary;
    assert_eq!(summary.status, PackStatus::PACK_STATUS_DRAFT);
    assert_eq!(summary.name, "はじめてのスタンプ");
    assert_eq!(summary.tags, vec!["cute", "ねこ"], "tags are normalized");
    assert_eq!(summary.short_id.len(), 11);
    assert_eq!(summary.creator.id, creator.id.to_string());
    assert!(!summary.official && summary.stamp_count == 0 && summary.cover.is_unset());
    let viewer = summary.viewer.as_option().unwrap();
    assert!(viewer.installed && viewer.can_manage && viewer.usable && !viewer.owned);
    assert_eq!(summary.install_count, 0, "the creator's own install is not counted");
    let review = pack.review.as_option().expect("the creator sees the review state");
    assert!(review.needs_review);

    // Reachable by UUID and short ID for the creator, invisible to everyone else.
    assert_eq!(ctx.get_pack(Some(&creator), &summary.short_id).await.unwrap().summary.id, summary.id);
    assert_eq!(ctx.get_pack(Some(&stranger), &summary.id).await.unwrap_err().reason, "PACK_NOT_FOUND");
    assert_eq!(ctx.get_pack(None, &summary.short_id).await.unwrap_err().reason, "PACK_NOT_FOUND");

    let mine: ListMyPacksResponse =
        ctx.call("Studio", "ListMyPacks", Some(&creator), &ListMyPacksRequest::default()).await.unwrap();
    assert_eq!(pack_names(&mine.packs), vec!["はじめてのスタンプ"]);
    let drafts_only = ListMyPacksRequest { status: PackStatus::PACK_STATUS_PUBLISHED.into(), ..Default::default() };
    let published: ListMyPacksResponse = ctx.call("Studio", "ListMyPacks", Some(&creator), &drafts_only).await.unwrap();
    assert!(published.packs.is_empty());
    let theirs: ListMyPacksResponse =
        ctx.call("Studio", "ListMyPacks", Some(&stranger), &ListMyPacksRequest::default()).await.unwrap();
    assert!(theirs.packs.is_empty());

    let anonymous = ctx
        .call::<_, CreatePackResponse>(
            "Studio",
            "CreatePack",
            None,
            &CreatePackRequest { name: "x".into(), ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!((anonymous.status, anonymous.reason.as_str()), (401, "SIGN_IN_REQUIRED"));
}

#[tokio::test]
async fn create_pack_validates_input_and_enforces_the_pack_limit() {
    let ctx = start_with(StampConfig { max_packs: 2, ..test_config() }).await;
    let creator = ctx.app.user("creator").await;
    let valid = || CreatePackRequest { name: "パック".into(), ..Default::default() };
    let cases: Vec<(&str, Box<dyn Fn(&mut CreatePackRequest)>)> = vec![
        ("PACK_NAME_INVALID", Box::new(|r| r.name = "  ".into())),
        ("PACK_NAME_INVALID", Box::new(|r| r.name = "あ".repeat(41))),
        ("PACK_DESCRIPTION_INVALID", Box::new(|r| r.description = "x".repeat(501))),
        ("TAGS_INVALID", Box::new(|r| r.tags = vec!["two words".into()])),
        ("TAGS_INVALID", Box::new(|r| r.tags = (0..11).map(|i| format!("t{i}")).collect())),
        ("PRICE_INVALID", Box::new(|r| r.price = 5)),
        ("PRICE_INVALID", Box::new(|r| r.price = 10_001)),
        ("FORBIDDEN", Box::new(|r| r.official = true)),
    ];
    for (reason, mutate) in cases {
        let mut request = valid();
        mutate(&mut request);
        assert_eq!(ctx.reason("Studio", "CreatePack", Some(&creator), &request).await, reason);
    }

    ctx.draft(&creator, "1つめ", 0, &[]).await;
    ctx.draft(&creator, "2つめ", 100, &[]).await;
    assert_eq!(ctx.reason("Studio", "CreatePack", Some(&creator), &valid()).await, "PACK_LIMIT_REACHED");
}

#[tokio::test]
async fn update_pack_changes_only_what_is_sent() {
    let ctx = start().await;
    let (creator, stranger) = (ctx.app.user("creator").await, ctx.app.user("stranger").await);
    let pack = ctx.draft(&creator, "もとの名前", 0, &["a"]).await;
    let id = id_of(&pack);
    let pack = ctx.add_stamps(&creator, &id, "s", 3).await;
    let first_image = pack.stamps[0].image.url.clone();
    assert_eq!(pack.summary.cover.url, first_image, "the first stamp is the cover by default");

    let rename =
        UpdatePackRequest { pack_id: id.clone(), name: Some("あたらしい名前".into()), ..Default::default() };
    let updated: UpdatePackResponse = ctx.call("Studio", "UpdatePack", Some(&creator), &rename).await.unwrap();
    assert_eq!(updated.pack.summary.name, "あたらしい名前");
    assert_eq!(updated.pack.description, "もとの名前 の説明", "untouched fields stay");
    assert_eq!(updated.pack.summary.tags, vec!["a"]);

    let third = pack.stamps[2].stamp_ref.id.clone();
    let everything = UpdatePackRequest {
        pack_id: id.clone(),
        description: Some("せつめい".into()),
        tags: Some(TagList { tags: vec!["B".into(), "c".into()], ..Default::default() }).into(),
        price: Some(120),
        cover_stamp_id: Some(third.clone()),
        ..Default::default()
    };
    let updated: UpdatePackResponse = ctx.call("Studio", "UpdatePack", Some(&creator), &everything).await.unwrap();
    assert_eq!((updated.pack.description.as_str(), updated.pack.summary.price), ("せつめい", 120));
    assert_eq!(updated.pack.summary.tags, vec!["b", "c"]);
    assert_eq!(updated.pack.cover_stamp_id, third);
    assert_eq!(updated.pack.summary.cover.url, pack.stamps[2].image.url);

    // An empty tag list clears the tags; an empty cover goes back to the first stamp.
    let clear = UpdatePackRequest {
        pack_id: id.clone(),
        tags: Some(TagList::default()).into(),
        cover_stamp_id: Some(String::new()),
        ..Default::default()
    };
    let updated: UpdatePackResponse = ctx.call("Studio", "UpdatePack", Some(&creator), &clear).await.unwrap();
    assert!(updated.pack.summary.tags.is_empty() && updated.pack.cover_stamp_id.is_empty());
    assert_eq!(updated.pack.summary.cover.url, first_image);

    let other_pack = ctx.draft(&creator, "べつのパック", 0, &[]).await;
    let other_stamp = ctx.add_stamps(&creator, &id_of(&other_pack), "o", 1).await.stamps[0].stamp_ref.id.clone();
    let foreign_cover =
        UpdatePackRequest { pack_id: id.clone(), cover_stamp_id: Some(other_stamp), ..Default::default() };
    assert_eq!(ctx.reason("Studio", "UpdatePack", Some(&creator), &foreign_cover).await, "COVER_INVALID");
    let bad_price = UpdatePackRequest { pack_id: id.clone(), price: Some(3), ..Default::default() };
    assert_eq!(ctx.reason("Studio", "UpdatePack", Some(&creator), &bad_price).await, "PRICE_INVALID");
    let bad_name = UpdatePackRequest { pack_id: id.clone(), name: Some(String::new()), ..Default::default() };
    assert_eq!(ctx.reason("Studio", "UpdatePack", Some(&creator), &bad_name).await, "PACK_NAME_INVALID");
    assert_eq!(ctx.reason("Studio", "UpdatePack", Some(&stranger), &rename).await, "PACK_NOT_FOUND");
    assert_eq!(ctx.reason("Studio", "UpdatePack", None, &rename).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn add_stamps_accepts_stills_animations_and_lottie() {
    let ctx = start().await;
    let (creator, stranger) = (ctx.app.user("creator").await, ctx.app.user("stranger").await);
    let id = id_of(&ctx.draft(&creator, "いろいろ", 0, &[]).await);

    let stamps = vec![
        NewStamp {
            asset_id: ctx.upload(&creator, Upload::Still).await,
            name: "やあ".into(),
            emoji: "👋".into(),
            keywords: vec!["Hello".into(), "こんにちは".into()],
            ..Default::default()
        },
        NewStamp {
            asset_id: ctx.upload(&creator, Upload::Animated).await,
            name: "ダンス".into(),
            ..Default::default()
        },
        NewStamp { asset_id: ctx.upload(&creator, Upload::Lottie).await, ..Default::default() },
    ];
    let added: AddStampsResponse = ctx
        .call(
            "Studio",
            "AddStamps",
            Some(&creator),
            &AddStampsRequest { pack_id: id.clone(), stamps, ..Default::default() },
        )
        .await
        .unwrap();
    let pack = added.pack.into_option().unwrap();
    assert_eq!(stamp_names(&pack.stamps), vec!["やあ", "ダンス", ""]);
    assert!(pack.summary.animated);
    assert_eq!(pack.summary.stamp_count, 3);
    assert_eq!(pack.review.pending_stamp_count, 3);

    let still = &pack.stamps[0];
    assert_eq!(still.stamp_ref.format, tokuly_proto::common::StampFormat::STAMP_FORMAT_STATIC);
    assert_eq!(still.stamp_ref.pack_id, id);
    assert!(still.stamp_ref.image_url.ends_with("/512.webp") && still.stamp_ref.animation_url.is_empty());
    assert_eq!((still.stamp_ref.width, still.stamp_ref.height), (512, 448));
    assert_eq!(still.image.variants.len(), 1, "the full processed image comes along");
    assert_eq!((still.emoji.as_str(), &still.keywords), ("👋", &vec!["hello".to_owned(), "こんにちは".to_owned()]));
    assert!(still.pending_review && still.usable && !still.personal);

    let animated = &pack.stamps[1];
    assert_eq!(animated.stamp_ref.format, tokuly_proto::common::StampFormat::STAMP_FORMAT_ANIMATED);
    assert!(animated.stamp_ref.image_url.ends_with("/512.webp") && animated.stamp_ref.animation_url.ends_with("/main"));
    let lottie = &pack.stamps[2];
    assert_eq!(lottie.stamp_ref.format, tokuly_proto::common::StampFormat::STAMP_FORMAT_LOTTIE);
    assert!(lottie.stamp_ref.image_url.is_empty() && lottie.stamp_ref.animation_url.ends_with("/main"));

    // More stamps go to the end.
    let pack = ctx.add_stamps(&creator, &id, "つづき", 2).await;
    assert_eq!(stamp_names(&pack.stamps), vec!["やあ", "ダンス", "", "つづき1", "つづき2"]);

    let not_mine = AddStampsRequest {
        pack_id: id.clone(),
        stamps: vec![ctx.new_stamp(&stranger, "x").await],
        ..Default::default()
    };
    assert_eq!(ctx.reason("Studio", "AddStamps", Some(&stranger), &not_mine).await, "PACK_NOT_FOUND");
}

#[tokio::test]
async fn add_stamps_rejects_bad_uploads_and_bad_text() {
    let ctx = start().await;
    let (creator, stranger) = (ctx.app.user("creator").await, ctx.app.user("stranger").await);
    let id = id_of(&ctx.draft(&creator, "けんしょう", 0, &[]).await);
    let add = |stamps: Vec<NewStamp>| AddStampsRequest { pack_id: id.clone(), stamps, ..Default::default() };
    let with_asset = |asset_id: String| NewStamp { asset_id, ..Default::default() };

    assert_eq!(ctx.reason("Studio", "AddStamps", Some(&creator), &add(vec![])).await, "STAMPS_REQUIRED");
    let too_many = (0..31).map(|_| with_asset(uuid::Uuid::now_v7().to_string())).collect();
    assert_eq!(ctx.reason("Studio", "AddStamps", Some(&creator), &add(too_many)).await, "TOO_MANY_STAMPS");

    let someone_elses = ctx.upload(&stranger, Upload::Still).await;
    assert_eq!(
        ctx.reason("Studio", "AddStamps", Some(&creator), &add(vec![with_asset(someone_elses)])).await,
        "ASSET_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason("Studio", "AddStamps", Some(&creator), &add(vec![with_asset(uuid::Uuid::now_v7().to_string())]))
            .await,
        "ASSET_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason("Studio", "AddStamps", Some(&creator), &add(vec![with_asset("nope".into())])).await,
        "INVALID_ID"
    );
    for (upload, reason) in [
        (Upload::ChatImage, "ASSET_WRONG_PURPOSE"),
        (Upload::Processing, "ASSET_NOT_READY"),
        (Upload::Video, "STAMP_ASSET_INVALID"),
    ] {
        let asset_id = ctx.upload(&creator, upload).await;
        assert_eq!(ctx.reason("Studio", "AddStamps", Some(&creator), &add(vec![with_asset(asset_id)])).await, reason);
    }

    let asset_id = ctx.upload(&creator, Upload::Still).await;
    let texts: Vec<(&str, NewStamp)> = vec![
        ("STAMP_NAME_INVALID", NewStamp { name: "な".repeat(41), ..with_asset(asset_id.clone()) }),
        ("STAMP_EMOJI_INVALID", NewStamp { emoji: "abc".into(), ..with_asset(asset_id.clone()) }),
        ("KEYWORDS_INVALID", NewStamp { keywords: vec!["has space".into()], ..with_asset(asset_id.clone()) }),
    ];
    for (reason, stamp) in texts {
        assert_eq!(ctx.reason("Studio", "AddStamps", Some(&creator), &add(vec![stamp])).await, reason);
    }
    assert_eq!(ctx.get_pack(Some(&creator), &id).await.unwrap().stamps.len(), 0, "a rejected request adds nothing");

    // A pack holds 120 stamps: fill it up and try one more.
    for batch in 1..=4 {
        ctx.add_stamps(&creator, &id, &format!("b{batch}-"), 30).await;
    }
    let one_more = add(vec![ctx.new_stamp(&creator, "121").await]);
    assert_eq!(ctx.reason("Studio", "AddStamps", Some(&creator), &one_more).await, "PACK_FULL");
}

#[tokio::test]
async fn stamps_can_be_renamed_reordered_and_removed() {
    let ctx = start().await;
    let (creator, stranger) = (ctx.app.user("creator").await, ctx.app.user("stranger").await);
    let id = id_of(&ctx.draft(&creator, "ならべかえ", 0, &[]).await);
    let pack = ctx.add_stamps(&creator, &id, "s", 3).await;
    let ids: Vec<String> = pack.stamps.iter().map(|s| s.stamp_ref.id.clone()).collect();

    let rename = UpdateStampRequest {
        stamp_id: ids[0].clone(),
        name: Some("おはよう".into()),
        emoji: Some("🌅".into()),
        ..Default::default()
    };
    let updated: UpdateStampResponse = ctx.call("Studio", "UpdateStamp", Some(&creator), &rename).await.unwrap();
    assert_eq!((updated.stamp.stamp_ref.name.as_str(), updated.stamp.emoji.as_str()), ("おはよう", "🌅"));
    let keywords_only = UpdateStampRequest {
        stamp_id: ids[0].clone(),
        keywords: Some(KeywordList { keywords: vec!["Morning".into()], ..Default::default() }).into(),
        ..Default::default()
    };
    let updated: UpdateStampResponse = ctx.call("Studio", "UpdateStamp", Some(&creator), &keywords_only).await.unwrap();
    assert_eq!(updated.stamp.keywords, vec!["morning"]);
    assert_eq!(updated.stamp.stamp_ref.name, "おはよう", "fields that were not sent stay");
    assert_eq!(ctx.reason("Studio", "UpdateStamp", Some(&stranger), &rename).await, "STAMP_NOT_FOUND");
    let bad_emoji = UpdateStampRequest { stamp_id: ids[0].clone(), emoji: Some("no".into()), ..Default::default() };
    assert_eq!(ctx.reason("Studio", "UpdateStamp", Some(&creator), &bad_emoji).await, "STAMP_EMOJI_INVALID");
    // Personal stamps are not edited through the studio.
    let personal = ctx.personal(&creator, "じぶん用").await;
    let personal_rename =
        UpdateStampRequest { stamp_id: personal.stamp_ref.id.clone(), name: Some("x".into()), ..Default::default() };
    assert_eq!(ctx.reason("Studio", "UpdateStamp", Some(&creator), &personal_rename).await, "STAMP_NOT_FOUND");

    let reversed: Vec<String> = ids.iter().rev().cloned().collect();
    let reorder = ReorderStampsRequest { pack_id: id.clone(), stamp_ids: reversed.clone(), ..Default::default() };
    let reordered: ReorderStampsResponse = ctx.call("Studio", "ReorderStamps", Some(&creator), &reorder).await.unwrap();
    assert_eq!(stamp_names(&reordered.pack.stamps), vec!["s3", "s2", "おはよう"]);
    assert_eq!(reordered.pack.summary.cover.url, pack.stamps[2].image.url, "the cover follows the first stamp");
    assert_eq!(reordered.pack.summary.previews.iter().map(|p| p.id.clone()).collect::<Vec<_>>(), reversed);
    let incomplete = ReorderStampsRequest { pack_id: id.clone(), stamp_ids: ids[..2].to_vec(), ..Default::default() };
    assert_eq!(ctx.reason("Studio", "ReorderStamps", Some(&creator), &incomplete).await, "ORDER_MISMATCH");
    assert_eq!(ctx.reason("Studio", "ReorderStamps", Some(&stranger), &reorder).await, "PACK_NOT_FOUND");

    let remove = RemoveStampRequest { stamp_id: ids[2].clone(), ..Default::default() };
    assert_eq!(ctx.reason("Studio", "RemoveStamp", Some(&stranger), &remove).await, "STAMP_NOT_FOUND");
    let removed: RemoveStampResponse = ctx.call("Studio", "RemoveStamp", Some(&creator), &remove).await.unwrap();
    assert_eq!(stamp_names(&removed.pack.stamps), vec!["s2", "おはよう"]);
    assert_eq!(removed.pack.summary.stamp_count, 2);
    assert_eq!(removed.pack.summary.cover.url, pack.stamps[1].image.url);
    assert_eq!(ctx.reason("Studio", "RemoveStamp", Some(&creator), &remove).await, "STAMP_NOT_FOUND");
}

#[tokio::test]
async fn packs_and_stamps_people_paid_for_cannot_be_deleted() {
    let ctx = start().await;
    let (creator, buyer, stranger) =
        (ctx.app.user("creator").await, ctx.app.user("buyer").await, ctx.app.user("stranger").await);

    // A draft nobody holds simply goes away, with its stamps.
    let draft = ctx.draft(&creator, "すてる", 0, &[]).await;
    let draft_id = id_of(&draft);
    let with_stamps = ctx.add_stamps(&creator, &draft_id, "s", 2).await;
    let stamp = stamp_uuid(&with_stamps, 0);
    // (A chosen cover points from the pack back at one of its stamps; deleting must cope with the cycle.)
    let choose_cover = UpdatePackRequest {
        pack_id: draft_id.clone(),
        cover_stamp_id: Some(with_stamps.stamps[1].stamp_ref.id.clone()),
        ..Default::default()
    };
    let _: UpdatePackResponse = ctx.call("Studio", "UpdatePack", Some(&creator), &choose_cover).await.unwrap();
    let delete = DeletePackRequest { pack_id: draft_id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Studio", "DeletePack", Some(&stranger), &delete).await, "PACK_NOT_FOUND");
    let _: DeletePackResponse = ctx.call("Studio", "DeletePack", Some(&creator), &delete).await.unwrap();
    assert_eq!(ctx.get_pack(Some(&creator), &draft_id).await.unwrap_err().reason, "PACK_NOT_FOUND");
    assert!(tokuly_stamp::stamp_refs(&ctx.app.core, &[stamp]).await.unwrap().is_empty());
    assert_eq!(ctx.app.events_by("stamp.pack.deleted", creator.id).await.len(), 1);

    // A published pack must keep at least one stamp.
    let single = ctx.published(&creator, "ひとつだけ", 0, &[], 1).await;
    let last = RemoveStampRequest { stamp_id: single.stamps[0].stamp_ref.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Studio", "RemoveStamp", Some(&creator), &last).await, "PACK_EMPTY");

    // Once somebody bought a pack, neither it nor its stamps can be removed.
    let paid = ctx.published(&creator, "ゆうりょう", 100, &[], 2).await;
    let paid_id = id_of(&paid);
    ctx.grant_coins(&buyer, 100).await;
    ctx.buy(&buyer, &paid_id, 100).await;
    assert_eq!(
        ctx.reason(
            "Studio",
            "DeletePack",
            Some(&creator),
            &DeletePackRequest { pack_id: paid_id.clone(), ..Default::default() }
        )
        .await,
        "PACK_HAS_BUYERS"
    );
    let remove = RemoveStampRequest { stamp_id: paid.stamps[0].stamp_ref.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Studio", "RemoveStamp", Some(&creator), &remove).await, "PACK_HAS_BUYERS");
    // It can still leave the store; the buyer keeps it.
    ctx.unpublish(&creator, &paid_id).await;
    assert_eq!(ctx.get_pack(Some(&buyer), &paid_id).await.unwrap().stamps.len(), 2);
    // A stamp added afterwards was never sold, so it can be removed again.
    let extra = ctx.add_stamps(&creator, &paid_id, "あと", 1).await;
    ctx.remove_stamp(&creator, &extra.stamps[2].stamp_ref.id).await;
}
