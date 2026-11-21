//! Personal ("quick") stamps and saving stamps other people sent.
mod common;

use common::*;
use tokuly_proto::stamp::*;
use tokuly_stamp::StampConfig;

#[tokio::test]
async fn personal_stamps_are_private_and_managed_by_their_owner() {
    let ctx = start_with(StampConfig { max_personal_stamps: 3, ..test_config() }).await;
    let (owner, stranger) = (ctx.app.user("owner").await, ctx.app.user("stranger").await);

    let first = ctx.personal(&owner, "いち").await;
    assert!(first.personal && first.usable && !first.pending_review);
    assert_eq!(first.stamp_ref.pack_id, "");
    assert!(first.stamp_ref.image_url.ends_with("/512.webp"));
    let animated = CreatePersonalStampRequest {
        asset_id: ctx.upload(&owner, Upload::Animated).await,
        name: "に".into(),
        emoji: "🎉".into(),
        keywords: vec!["Party".into()],
        ..Default::default()
    };
    let second: CreatePersonalStampResponse =
        ctx.call("Library", "CreatePersonalStamp", Some(&owner), &animated).await.unwrap();
    assert_eq!(second.stamp.stamp_ref.format, tokuly_proto::common::StampFormat::STAMP_FORMAT_ANIMATED);
    assert_eq!((second.stamp.emoji.as_str(), second.stamp.keywords.clone()), ("🎉", vec!["party".to_owned()]));
    let third = ctx.personal(&owner, "さん").await;

    let list = || async {
        let response: ListPersonalStampsResponse = ctx
            .call("Library", "ListPersonalStamps", Some(&owner), &ListPersonalStampsRequest::default())
            .await
            .unwrap();
        stamp_names(&response.stamps)
    };
    assert_eq!(list().await, vec!["さん", "に", "いち"], "new stamps go first");
    let theirs: ListPersonalStampsResponse = ctx
        .call("Library", "ListPersonalStamps", Some(&stranger), &ListPersonalStampsRequest::default())
        .await
        .unwrap();
    assert!(theirs.stamps.is_empty());

    // Only the owner can send, edit, reorder or delete them.
    let core = &ctx.app.core;
    let first_id: uuid::Uuid = first.stamp_ref.id.parse().unwrap();
    assert!(tokuly_stamp::ensure_usable(core, owner.id, first_id).await.is_ok());
    assert_eq!(tokuly_stamp::ensure_usable(core, stranger.id, first_id).await.unwrap_err().reason, "STAMP_NOT_USABLE");

    let rename = UpdatePersonalStampRequest {
        stamp_id: first.stamp_ref.id.clone(),
        name: Some("いちばん".into()),
        ..Default::default()
    };
    assert_eq!(ctx.reason("Library", "UpdatePersonalStamp", Some(&stranger), &rename).await, "STAMP_NOT_FOUND");
    let renamed: UpdatePersonalStampResponse =
        ctx.call("Library", "UpdatePersonalStamp", Some(&owner), &rename).await.unwrap();
    assert_eq!(renamed.stamp.stamp_ref.name, "いちばん");
    let bad = UpdatePersonalStampRequest {
        stamp_id: first.stamp_ref.id.clone(),
        keywords: Some(KeywordList { keywords: vec!["a b".into()], ..Default::default() }).into(),
        ..Default::default()
    };
    assert_eq!(ctx.reason("Library", "UpdatePersonalStamp", Some(&owner), &bad).await, "KEYWORDS_INVALID");

    let order = vec![first.stamp_ref.id.clone(), third.stamp_ref.id.clone(), second.stamp.stamp_ref.id.clone()];
    let _: ReorderPersonalStampsResponse = ctx
        .call(
            "Library",
            "ReorderPersonalStamps",
            Some(&owner),
            &ReorderPersonalStampsRequest { stamp_ids: order.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(list().await, vec!["いちばん", "さん", "に"]);
    let partial = ReorderPersonalStampsRequest { stamp_ids: order[..2].to_vec(), ..Default::default() };
    assert_eq!(ctx.reason("Library", "ReorderPersonalStamps", Some(&owner), &partial).await, "ORDER_MISMATCH");
    assert_eq!(
        ctx.reason(
            "Library",
            "ReorderPersonalStamps",
            Some(&stranger),
            &ReorderPersonalStampsRequest { stamp_ids: order, ..Default::default() }
        )
        .await,
        "ORDER_MISMATCH"
    );

    // The limit, then deleting to make room.
    let fourth = CreatePersonalStampRequest { asset_id: ctx.upload(&owner, Upload::Still).await, ..Default::default() };
    assert_eq!(
        ctx.reason("Library", "CreatePersonalStamp", Some(&owner), &fourth).await,
        "PERSONAL_STAMP_LIMIT_REACHED"
    );
    let delete = DeletePersonalStampRequest { stamp_id: third.stamp_ref.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Library", "DeletePersonalStamp", Some(&stranger), &delete).await, "STAMP_NOT_FOUND");
    let _: DeletePersonalStampResponse =
        ctx.call("Library", "DeletePersonalStamp", Some(&owner), &delete).await.unwrap();
    assert_eq!(ctx.reason("Library", "DeletePersonalStamp", Some(&owner), &delete).await, "STAMP_NOT_FOUND");
    assert!(
        tokuly_stamp::stamp_refs(core, &[third.stamp_ref.id.parse().unwrap()]).await.unwrap().is_empty(),
        "deleted everywhere"
    );
    let _: CreatePersonalStampResponse =
        ctx.call("Library", "CreatePersonalStamp", Some(&owner), &fourth).await.unwrap();

    // Uploads are checked like in the studio.
    let cases = [
        (Upload::ChatImage, "ASSET_WRONG_PURPOSE"),
        (Upload::Processing, "ASSET_NOT_READY"),
        (Upload::Video, "STAMP_ASSET_INVALID"),
    ];
    for (upload, reason) in cases {
        let request =
            CreatePersonalStampRequest { asset_id: ctx.upload(&stranger, upload).await, ..Default::default() };
        assert_eq!(ctx.reason("Library", "CreatePersonalStamp", Some(&stranger), &request).await, reason);
    }
    let not_mine =
        CreatePersonalStampRequest { asset_id: ctx.upload(&owner, Upload::Still).await, ..Default::default() };
    assert_eq!(ctx.reason("Library", "CreatePersonalStamp", Some(&stranger), &not_mine).await, "ASSET_NOT_FOUND");
    let bad_emoji = CreatePersonalStampRequest {
        asset_id: ctx.upload(&stranger, Upload::Still).await,
        emoji: "xx".into(),
        ..Default::default()
    };
    assert_eq!(ctx.reason("Library", "CreatePersonalStamp", Some(&stranger), &bad_emoji).await, "STAMP_EMOJI_INVALID");
    assert_eq!(ctx.reason("Library", "CreatePersonalStamp", None, &not_mine).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn saving_a_received_stamp_copies_personal_stamps_and_installs_packs() {
    let ctx = start().await;
    let (alice, bob, carol) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.app.user("carol").await);
    let save = |stamp_id: &str| SaveStampRequest { stamp_id: stamp_id.to_owned(), ..Default::default() };
    let get = |stamp_id: &str| GetStampRequest { stamp_id: stamp_id.to_owned(), ..Default::default() };

    // Alice's personal stamp, received by Bob: he gets a copy of his own.
    let hers = ctx.personal(&alice, "アリスの").await;
    let before: GetStampResponse = ctx.call("Store", "GetStamp", Some(&bob), &get(&hers.stamp_ref.id)).await.unwrap();
    assert!(before.can_save && !before.stamp.usable && before.pack.is_unset());
    let saved: SaveStampResponse =
        ctx.call("Library", "SaveStamp", Some(&bob), &save(&hers.stamp_ref.id)).await.unwrap();
    assert_eq!(saved.outcome, SaveOutcome::SAVE_OUTCOME_SAVED_AS_PERSONAL);
    let copy = saved.stamp.into_option().unwrap();
    assert_ne!(copy.stamp_ref.id, hers.stamp_ref.id);
    assert_eq!(
        (copy.stamp_ref.name.as_str(), copy.stamp_ref.image_url.as_str()),
        ("アリスの", hers.stamp_ref.image_url.as_str())
    );
    assert!(copy.personal && copy.usable);
    assert!(tokuly_stamp::ensure_usable(&ctx.app.core, bob.id, copy.stamp_ref.id.parse().unwrap()).await.is_ok());

    // Saving again — the original or his copy — changes nothing.
    for id in [&hers.stamp_ref.id, &copy.stamp_ref.id] {
        let again: SaveStampResponse = ctx.call("Library", "SaveStamp", Some(&bob), &save(id)).await.unwrap();
        assert_eq!(again.outcome, SaveOutcome::SAVE_OUTCOME_ALREADY_IN_LIBRARY);
        assert_eq!(again.stamp.stamp_ref.id, copy.stamp_ref.id);
    }
    let after: GetStampResponse = ctx.call("Store", "GetStamp", Some(&bob), &get(&hers.stamp_ref.id)).await.unwrap();
    assert!(!after.can_save);
    // Carol receives Bob's copy and saves it; Alice already has the original.
    let carols: SaveStampResponse =
        ctx.call("Library", "SaveStamp", Some(&carol), &save(&copy.stamp_ref.id)).await.unwrap();
    assert_eq!(carols.outcome, SaveOutcome::SAVE_OUTCOME_SAVED_AS_PERSONAL);
    let alices: SaveStampResponse =
        ctx.call("Library", "SaveStamp", Some(&alice), &save(&copy.stamp_ref.id)).await.unwrap();
    assert_eq!(alices.outcome, SaveOutcome::SAVE_OUTCOME_ALREADY_IN_LIBRARY);
    assert_eq!(alices.stamp.stamp_ref.id, hers.stamp_ref.id);
    // Alice deleting hers does not take Bob's copy away.
    let _: DeletePersonalStampResponse = ctx
        .call(
            "Library",
            "DeletePersonalStamp",
            Some(&alice),
            &DeletePersonalStampRequest { stamp_id: hers.stamp_ref.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(tokuly_stamp::ensure_usable(&ctx.app.core, bob.id, copy.stamp_ref.id.parse().unwrap()).await.is_ok());

    // A stamp from a free pack installs the pack.
    let free = ctx.published(&alice, "むりょう", 0, &[], 2).await;
    let from_free = &free.stamps[1].stamp_ref.id;
    let preview: GetStampResponse = ctx.call("Store", "GetStamp", Some(&bob), &get(from_free)).await.unwrap();
    assert!(preview.can_save && preview.stamp.usable);
    assert_eq!(preview.pack.id, id_of(&free));
    let saved: SaveStampResponse = ctx.call("Library", "SaveStamp", Some(&bob), &save(from_free)).await.unwrap();
    assert_eq!(saved.outcome, SaveOutcome::SAVE_OUTCOME_PACK_INSTALLED);
    assert!(saved.pack.viewer.installed);
    assert_eq!(saved.stamp.stamp_ref.id, *from_free);
    let again: SaveStampResponse = ctx.call("Library", "SaveStamp", Some(&bob), &save(from_free)).await.unwrap();
    assert_eq!(again.outcome, SaveOutcome::SAVE_OUTCOME_ALREADY_IN_LIBRARY);

    // A stamp from a paid pack points at the store instead of being copied out.
    let paid = ctx.published(&alice, "ゆうりょう", 100, &[], 1).await;
    let from_paid = &paid.stamps[0].stamp_ref.id;
    let saved: SaveStampResponse = ctx.call("Library", "SaveStamp", Some(&bob), &save(from_paid)).await.unwrap();
    assert_eq!(saved.outcome, SaveOutcome::SAVE_OUTCOME_PURCHASE_REQUIRED);
    assert_eq!((saved.pack.id.clone(), saved.pack.price), (id_of(&paid), 100));
    assert!(!saved.stamp.usable);
    let preview: GetStampResponse = ctx.call("Store", "GetStamp", Some(&bob), &get(from_paid)).await.unwrap();
    assert!(!preview.can_save);

    // A reviewed stamp of a pack that left the store cannot be had any more.
    ctx.unpublish(&alice, &id_of(&paid)).await;
    assert_eq!(ctx.reason("Library", "SaveStamp", Some(&bob), &save(from_paid)).await, "STAMP_NOT_SAVABLE");
    // A stamp nobody reviewed does not exist for other people.
    let draft = ctx.draft(&alice, "したがき", 0, &[]).await;
    let unreviewed = ctx.add_stamps(&alice, &id_of(&draft), "s", 1).await.stamps[0].stamp_ref.id.clone();
    assert_eq!(ctx.reason("Library", "SaveStamp", Some(&bob), &save(&unreviewed)).await, "STAMP_NOT_FOUND");
    assert_eq!(ctx.reason("Store", "GetStamp", Some(&bob), &get(&unreviewed)).await, "STAMP_NOT_FOUND");
    assert_eq!(
        ctx.reason("Library", "SaveStamp", Some(&bob), &save(&uuid::Uuid::now_v7().to_string())).await,
        "STAMP_NOT_FOUND"
    );
    assert_eq!(ctx.reason("Library", "SaveStamp", None, &save(from_free)).await, "SIGN_IN_REQUIRED");
}
