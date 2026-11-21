//! Stamps and the uploads they are made from: a stamp holds its upload in
//! the media domain for as long as it exists, and only a forced deletion of
//! the upload takes stamps away.
mod common;

use common::*;
use tokuly_media::assets;
use tokuly_proto::stamp::*;
use tokuly_stamp::stamp_refs;
use uuid::Uuid;

/// Deletes an upload whatever holds it, as a takedown does, and returns the
/// event with which the media domain announces it.
async fn force_delete(ctx: &Ctx, asset_id: Uuid) -> tokuly_core::events::Event {
    assert!(assets::delete(&ctx.app.core, asset_id, None).await.unwrap());
    let events = ctx.app.events("media.asset.deleted").await;
    events
        .into_iter()
        .find(|event| event.data["asset_id"] == asset_id.to_string())
        .expect("media announces the deletion")
}

#[tokio::test]
async fn personal_stamps_and_saved_copies_each_hold_the_upload() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let core = &ctx.app.core;

    let original = ctx.personal(&alice, "しゃしん").await;
    let upload = asset_of(&original);
    assert_eq!(ctx.holders(upload).await, vec![hold_of(&original)]);
    // An upload in use cannot be deleted by its owner.
    assert_eq!(assets::delete(core, upload, Some(alice.id)).await.unwrap_err().reason, "ASSET_IN_USE");

    // Bob's copy holds the same upload in its own right…
    let copy = ctx.save(&bob, &original.stamp_ref.id).await.stamp.into_option().unwrap();
    assert_eq!(asset_of(&copy), upload);
    let mut both = vec![hold_of(&original), hold_of(&copy)];
    both.sort();
    assert_eq!(ctx.holders(upload).await, both);
    // …and saving once more adds neither a stamp nor a hold.
    ctx.save(&bob, &original.stamp_ref.id).await;
    assert_eq!(ctx.holders(upload).await.len(), 2);

    // …so it survives the original, and keeps the upload alive on its own.
    let delete =
        |stamp: &Stamp| DeletePersonalStampRequest { stamp_id: stamp.stamp_ref.id.clone(), ..Default::default() };
    let _: DeletePersonalStampResponse =
        ctx.call("Library", "DeletePersonalStamp", Some(&alice), &delete(&original)).await.unwrap();
    assert_eq!(ctx.holders(upload).await, vec![hold_of(&copy)]);
    assert_eq!(assets::delete(core, upload, Some(alice.id)).await.unwrap_err().reason, "ASSET_IN_USE");
    assert!(stamp_refs(core, &[copy.stamp_ref.id.parse().unwrap()]).await.unwrap().len() == 1);

    // Once the last stamp is gone the upload is free again (and would be collected in time).
    let _: DeletePersonalStampResponse =
        ctx.call("Library", "DeletePersonalStamp", Some(&bob), &delete(&copy)).await.unwrap();
    assert!(ctx.holders(upload).await.is_empty());
    assert!(!assets::is_attached(core, upload).await.unwrap());
    assert!(
        assets::delete(core, upload, Some(alice.id)).await.unwrap(),
        "nothing holds it, so its owner may delete it"
    );
}

#[tokio::test]
async fn pack_stamps_hold_their_uploads_until_they_or_the_pack_are_deleted() {
    let ctx = start().await;
    let creator = ctx.app.user("creator").await;
    let core = &ctx.app.core;
    let id = id_of(&ctx.draft(&creator, "アップロード", 0, &[]).await);

    // A request that is rejected holds nothing, even for the uploads that were fine.
    let fine = ctx.upload(&creator, Upload::Still).await;
    let stamps = vec![
        NewStamp { asset_id: fine.clone(), ..Default::default() },
        NewStamp { asset_id: ctx.upload(&creator, Upload::Video).await, ..Default::default() },
    ];
    let rejected = AddStampsRequest { pack_id: id.clone(), stamps, ..Default::default() };
    assert_eq!(ctx.reason("Studio", "AddStamps", Some(&creator), &rejected).await, "STAMP_ASSET_INVALID");
    assert!(ctx.holders(fine.parse().unwrap()).await.is_empty());

    let pack = ctx.add_stamps(&creator, &id, "s", 3).await;
    for stamp in &pack.stamps {
        assert_eq!(ctx.holders(asset_of(stamp)).await, vec![hold_of(stamp)]);
        assert_eq!(assets::delete(core, asset_of(stamp), Some(creator.id)).await.unwrap_err().reason, "ASSET_IN_USE");
    }

    // Removing a stamp lets go of its upload only.
    ctx.remove_stamp(&creator, &pack.stamps[0].stamp_ref.id).await;
    assert!(ctx.holders(asset_of(&pack.stamps[0])).await.is_empty());
    assert_eq!(ctx.holders(asset_of(&pack.stamps[1])).await, vec![hold_of(&pack.stamps[1])]);

    // Deleting the pack lets go of the rest.
    ctx.delete_pack(&creator, &id).await;
    for stamp in &pack.stamps {
        assert!(ctx.holders(asset_of(stamp)).await.is_empty());
    }
}

#[tokio::test]
async fn a_forced_upload_deletion_removes_every_stamp_that_showed_it() {
    let ctx = start().await;
    let (alice, bob) = (ctx.app.user("alice").await, ctx.app.user("bob").await);
    let core = &ctx.app.core;
    core.storage.ensure_buckets().await.unwrap();

    // One upload used three times: a personal stamp, Bob's saved copy of it, and a stamp of a pack somebody bought.
    let asset_id = ctx.upload(&alice, Upload::Still).await;
    let request =
        CreatePersonalStampRequest { asset_id: asset_id.clone(), name: "しゃしん".into(), ..Default::default() };
    let personal: CreatePersonalStampResponse =
        ctx.call("Library", "CreatePersonalStamp", Some(&alice), &request).await.unwrap();
    let copy = ctx.save(&bob, &personal.stamp.stamp_ref.id).await;
    let id = id_of(&ctx.draft(&alice, "おなじ絵", 100, &[]).await);
    let same_picture = NewStamp { asset_id: asset_id.clone(), name: "おなじ".into(), ..Default::default() };
    let add = AddStampsRequest { pack_id: id.clone(), stamps: vec![same_picture], ..Default::default() };
    let _: AddStampsResponse = ctx.call("Studio", "AddStamps", Some(&alice), &add).await.unwrap();
    let pack = ctx.add_stamps(&alice, &id, "べつの絵", 1).await;
    ctx.submit(&alice, &id).await;
    ctx.approve(&id).await;
    ctx.grant_coins(&bob, 100).await;
    ctx.buy(&bob, &id, 100).await;
    let upload: Uuid = asset_id.parse().unwrap();
    assert_eq!(ctx.holders(upload).await.len(), 3);

    // Its uploader cannot take the picture away from the people who hold it — least of all from a buyer.
    assert_eq!(assets::delete(core, upload, Some(alice.id)).await.unwrap_err().reason, "ASSET_IN_USE");
    let remove = RemoveStampRequest { stamp_id: pack.stamps[0].stamp_ref.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Studio", "RemoveStamp", Some(&alice), &remove).await, "PACK_HAS_BUYERS");
    let showed_it: Vec<Uuid> = [&personal.stamp.stamp_ref.id, &copy.stamp.stamp_ref.id, &pack.stamps[0].stamp_ref.id]
        .iter()
        .map(|id| id.parse().unwrap())
        .collect();
    assert_eq!(stamp_refs(core, &showed_it).await.unwrap().len(), 3);

    // A forced deletion is the one thing that does. The stamps follow, so none is left with a broken picture.
    let deleted = force_delete(&ctx, upload).await;
    ctx.stamp.handle_event(deleted.clone()).await.unwrap();
    ctx.stamp.handle_event(deleted).await.unwrap();
    assert!(stamp_refs(core, &showed_it).await.unwrap().is_empty());
    // The pack carries on with the stamp it has left, cover included.
    let after = ctx.get_pack(None, &id).await.unwrap();
    assert_eq!(stamp_names(&after.stamps), vec!["べつの絵1"]);
    assert_eq!(after.summary.cover.url, pack.stamps[1].image.url);
    assert_eq!(after.summary.status, PackStatus::PACK_STATUS_PUBLISHED);
    assert!(
        ctx.notifications(&alice, "stamp.stamp.removed").await.is_empty(),
        "whoever forced the deletion does the telling"
    );

    // Losing its last picture takes the pack out of the store; its buyer still has the (now empty) pack.
    let last = force_delete(&ctx, asset_of(&after.stamps[0])).await;
    ctx.stamp.handle_event(last).await.unwrap();
    assert_eq!(ctx.get_pack(None, &id).await.unwrap_err().reason, "PACK_NOT_FOUND");
    assert_eq!(ctx.get_pack(Some(&bob), &id).await.unwrap().summary.status, PackStatus::PACK_STATUS_DRAFT);
    // Uploads no stamp was made from are none of our business.
    let unrelated = event("media.asset.deleted", serde_json::json!({ "asset_id": Uuid::now_v7() }));
    ctx.stamp.handle_event(unrelated).await.unwrap();
}
