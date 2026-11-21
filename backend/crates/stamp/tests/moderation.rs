//! Moderation: the review queue, approving and rejecting, trusted creators,
//! and takedowns of packs and single stamps.
mod common;

use common::*;
use tokuly_core::events::Event;
use tokuly_proto::{common::SubjectType, stamp::*};
use tokuly_stamp::{ensure_usable, stamp_refs};
use uuid::Uuid;

/// `admin.content.removed` for a pack or a stamp.
fn content_removed(subject_type: SubjectType, subject_id: &str, moderator: &tokuly_testkit::TestUser) -> Event {
    event(
        "admin.content.removed",
        serde_json::json!({ "subject_type": subject_type as i32, "subject_id": subject_id, "reason": "権利侵害", "moderator_id": moderator.id }),
    )
}

async fn queue(ctx: &Ctx, moderator: &tokuly_testkit::TestUser) -> ListReviewQueueResponse {
    let request = ListReviewQueueRequest {
        page: Some(tokuly_proto::common::PageRequest { limit: 30, ..Default::default() }).into(),
        ..Default::default()
    };
    ctx.call("Review", "ListReviewQueue", Some(moderator), &request).await.unwrap()
}

#[tokio::test]
async fn moderators_approve_packs_from_the_review_queue() {
    let ctx = start().await;
    let (creator, moderator) = (ctx.app.user("creator").await, ctx.moderator().await);
    let id = id_of(&ctx.draft(&creator, "しんさしてね", 0, &["review"]).await);
    ctx.add_stamps(&creator, &id, "s", 2).await;
    ctx.submit(&creator, &id).await;

    assert_eq!(
        ctx.reason("Review", "ListReviewQueue", Some(&creator), &ListReviewQueueRequest::default()).await,
        "FORBIDDEN"
    );
    assert_eq!(
        ctx.reason("Review", "ListReviewQueue", None, &ListReviewQueueRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );
    let waiting = queue(&ctx, &moderator).await;
    assert!(waiting.pending_count >= 1);
    let item = waiting.items.iter().find(|item| item.pack.summary.id == id).expect("the pack is in the queue");
    assert_eq!(item.pack.stamps.len(), 2, "moderators see the stamps they are judging");
    assert!(item.pack.stamps.iter().all(|s| s.pending_review));
    assert!(item.history.is_empty() && item.pack.review.needs_review);

    let approve = ApprovePackRequest { pack_id: id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Review", "ApprovePack", Some(&creator), &approve).await, "FORBIDDEN");
    let approved: ApprovePackResponse = ctx.call("Review", "ApprovePack", Some(&moderator), &approve).await.unwrap();
    assert_eq!(approved.pack.summary.status, PackStatus::PACK_STATUS_PUBLISHED);
    assert!(approved.pack.stamps.iter().all(|s| !s.pending_review));
    assert!(approved.pack.review.reviewed_at.is_set());
    assert_eq!(ctx.reason("Review", "ApprovePack", Some(&moderator), &approve).await, "PACK_NOT_IN_REVIEW");
    assert_eq!(
        ctx.reason(
            "Review",
            "ApprovePack",
            Some(&moderator),
            &ApprovePackRequest { pack_id: Uuid::now_v7().to_string(), ..Default::default() }
        )
        .await,
        "PACK_NOT_FOUND"
    );
    assert!(queue(&ctx, &moderator).await.items.iter().all(|item| item.pack.summary.id != id));

    // The store, the creator, their friends and the search index all hear about it.
    assert!(ctx.get_pack(None, &id).await.is_ok());
    let notices = ctx.notifications(&creator, "stamp.pack.approved").await;
    assert_eq!(notices.len(), 1);
    assert!(notices[0]["body"]["ja"].as_str().unwrap().contains("しんさしてね"));
    assert!(notices[0]["actor_id"].is_null(), "creators are not told which moderator decided");
    let activity = ctx.app.events_by("activity.recorded", creator.id).await;
    assert_eq!(activity.len(), 1);
    assert_eq!(
        (activity[0].data["kind"].as_str(), activity[0].data["audience"].as_str()),
        (Some("stamp.pack.published"), Some("public"))
    );
    let indexed = ctx.app.events("search.changed").await.into_iter().find(|e| e.data["id"] == id).unwrap();
    assert_eq!(indexed.data["doc"]["kind"], "stamp_pack");
    assert_eq!(indexed.data["doc"]["title"], "しんさしてね");
    assert_eq!(indexed.data["doc"]["tags"][0], "review");
    assert_eq!(ctx.app.events_by("stamp.pack.published", moderator.id).await.len(), 1);
}

#[tokio::test]
async fn rejected_packs_go_back_to_their_creator_with_a_reason() {
    let ctx = start().await;
    let (creator, moderator) = (ctx.app.user("creator").await, ctx.moderator().await);
    let id = id_of(&ctx.draft(&creator, "だめかも", 0, &[]).await);
    let pack = ctx.add_stamps(&creator, &id, "s", 2).await;
    ctx.submit(&creator, &id).await;

    let reject =
        |reason: &str| RejectPackRequest { pack_id: id.clone(), reason: reason.to_owned(), ..Default::default() };
    assert_eq!(ctx.reason("Review", "RejectPack", Some(&moderator), &reject("  ")).await, "REASON_REQUIRED");
    assert_eq!(ctx.reason("Review", "RejectPack", Some(&creator), &reject("x")).await, "FORBIDDEN");
    let rejected: RejectPackResponse =
        ctx.call("Review", "RejectPack", Some(&moderator), &reject("2枚目が他の作品に似ています")).await.unwrap();
    assert_eq!(rejected.pack.summary.status, PackStatus::PACK_STATUS_REJECTED);
    assert_eq!(ctx.reason("Review", "RejectPack", Some(&moderator), &reject("again")).await, "PACK_NOT_IN_REVIEW");

    let seen = ctx.get_pack(Some(&creator), &id).await.unwrap();
    assert_eq!(seen.review.note, "2枚目が他の作品に似ています");
    assert_eq!(ctx.get_pack(None, &id).await.unwrap_err().reason, "PACK_NOT_FOUND");
    let notices = ctx.notifications(&creator, "stamp.pack.rejected").await;
    assert_eq!(notices.len(), 1);
    assert!(notices[0]["body"]["en"].as_str().unwrap().contains("2枚目が他の作品に似ています"));
    assert_eq!(ctx.app.events_by("stamp.pack.rejected", moderator.id).await.len(), 1);

    // The creator fixes it and submits again; the next reviewer sees the history.
    ctx.remove_stamp(&creator, &pack.stamps[1].stamp_ref.id).await;
    assert_eq!(
        ctx.submit(&creator, &id).await.summary.status,
        PackStatus::PACK_STATUS_IN_REVIEW,
        "a rejected pack is always looked at again"
    );
    let waiting = queue(&ctx, &moderator).await;
    let item = waiting.items.iter().find(|item| item.pack.summary.id == id).unwrap();
    assert_eq!(item.history.len(), 1);
    assert_eq!(item.history[0].decision, ReviewDecision::REVIEW_DECISION_REJECTED);
    assert_eq!(item.history[0].moderator.id, moderator.id.to_string());
    let approved = ctx.approve(&id).await;
    assert_eq!(approved.review.note, "", "the old reason is cleared once it is live");
}

#[tokio::test]
async fn moderators_flag_trusted_creators() {
    let ctx = start().await;
    let (creator, moderator) = (ctx.app.user("creator").await, ctx.moderator().await);
    let trust =
        |trusted: bool| SetCreatorTrustedRequest { user_id: creator.id.to_string(), trusted, ..Default::default() };
    assert_eq!(ctx.reason("Review", "SetCreatorTrusted", Some(&creator), &trust(true)).await, "FORBIDDEN");
    let nobody = SetCreatorTrustedRequest { user_id: Uuid::now_v7().to_string(), trusted: true, ..Default::default() };
    assert_eq!(ctx.reason("Review", "SetCreatorTrusted", Some(&moderator), &nobody).await, "USER_NOT_FOUND");

    let _: SetCreatorTrustedResponse =
        ctx.call("Review", "SetCreatorTrusted", Some(&moderator), &trust(true)).await.unwrap();
    let _: SetCreatorTrustedResponse =
        ctx.call("Review", "SetCreatorTrusted", Some(&moderator), &trust(true)).await.unwrap();
    let changes = ctx
        .app
        .events("stamp.creator.trust_changed")
        .await
        .into_iter()
        .filter(|e| e.data["user_id"] == creator.id.to_string())
        .count();
    assert_eq!(changes, 1, "setting the same value twice changes nothing");
    assert_eq!(ctx.notifications(&creator, "stamp.creator.trusted").await.len(), 1);

    // Losing the flag sends the next submission back to review.
    let id = id_of(&ctx.draft(&creator, "しんらい", 0, &[]).await);
    ctx.add_stamps(&creator, &id, "s", 1).await;
    let _: SetCreatorTrustedResponse =
        ctx.call("Review", "SetCreatorTrusted", Some(&moderator), &trust(false)).await.unwrap();
    assert_eq!(ctx.submit(&creator, &id).await.summary.status, PackStatus::PACK_STATUS_IN_REVIEW);
}

#[tokio::test]
async fn a_takedown_removes_a_pack_for_everyone_until_it_is_approved_again() {
    let ctx = start().await;
    let (creator, fan, moderator) = (ctx.app.user("creator").await, ctx.app.user("fan").await, ctx.moderator().await);
    let core = &ctx.app.core;
    let pack = ctx.published(&creator, "もんだいあり", 0, &[], 2).await;
    let id = id_of(&pack);
    let stamp = stamp_uuid(&pack, 0);
    ctx.install(&fan, &id).await;
    // Even a trusted creator's takedown is reviewed again.
    ctx.trust(&moderator, &creator, true).await;

    let removal = content_removed(SubjectType::SUBJECT_TYPE_STAMP_PACK, &id, &moderator);
    ctx.stamp.handle_event(removal.clone()).await.unwrap();
    ctx.stamp.handle_event(removal).await.unwrap(); // redelivery is harmless

    // Gone from messages, from pickers and from the store.
    assert!(stamp_refs(core, &[stamp]).await.unwrap().is_empty());
    for user in [&fan, &creator] {
        assert_eq!(ensure_usable(core, user.id, stamp).await.unwrap_err().reason, "STAMP_NOT_USABLE");
    }
    assert_eq!(ctx.get_pack(Some(&fan), &id).await.unwrap_err().reason, "PACK_NOT_FOUND");
    let installed: ListInstalledPacksResponse =
        ctx.call("Library", "ListInstalledPacks", Some(&fan), &ListInstalledPacksRequest::default()).await.unwrap();
    assert!(installed.packs.is_empty());
    assert_eq!(
        ctx.reason(
            "Library",
            "InstallPack",
            Some(&fan),
            &InstallPackRequest { pack_id: id.clone(), ..Default::default() }
        )
        .await,
        "PACK_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason(
            "Store",
            "GetStamp",
            Some(&fan),
            &GetStampRequest { stamp_id: stamp.to_string(), ..Default::default() }
        )
        .await,
        "STAMP_NOT_FOUND"
    );
    assert_eq!(ctx.search_changes(&id).await, vec![true, false]);

    // The creator sees what happened and why, once.
    let seen = ctx.get_pack(Some(&creator), &id).await.unwrap();
    assert_eq!(seen.summary.status, PackStatus::PACK_STATUS_REMOVED);
    assert_eq!(seen.review.note, "権利侵害");
    assert_eq!(ctx.notifications(&creator, "stamp.pack.removed").await.len(), 1);
    let removed_events = ctx.app.events_by("stamp.pack.removed", moderator.id).await;
    assert_eq!(removed_events.len(), 1);
    assert_eq!(removed_events[0].data["reason"], "権利侵害");

    // Fixing and resubmitting goes through a moderator, trusted or not.
    assert!(seen.review.needs_review);
    assert_eq!(ctx.submit(&creator, &id).await.summary.status, PackStatus::PACK_STATUS_IN_REVIEW);
    let item = queue(&ctx, &moderator).await.items.into_iter().find(|item| item.pack.summary.id == id).unwrap();
    assert_eq!(item.history[0].decision, ReviewDecision::REVIEW_DECISION_REMOVED);
    ctx.approve(&id).await;
    assert!(stamp_refs(core, &[stamp]).await.unwrap().contains_key(&stamp));
    assert!(ensure_usable(core, fan.id, stamp).await.is_ok(), "the fan's install survived the takedown");

    // Events about other kinds of content, or unknown packs, are ignored.
    ctx.stamp.handle_event(content_removed(SubjectType::SUBJECT_TYPE_VIDEO, &id, &moderator)).await.unwrap();
    ctx.stamp
        .handle_event(content_removed(SubjectType::SUBJECT_TYPE_STAMP_PACK, &Uuid::now_v7().to_string(), &moderator))
        .await
        .unwrap();
    assert_eq!(ctx.get_pack(None, &id).await.unwrap().summary.status, PackStatus::PACK_STATUS_PUBLISHED);
}

#[tokio::test]
async fn a_single_stamp_can_be_taken_down_with_every_saved_copy() {
    let ctx = start().await;
    let (alice, bob, moderator) = (ctx.app.user("alice").await, ctx.app.user("bob").await, ctx.moderator().await);
    let core = &ctx.app.core;

    // A personal stamp and the copy Bob saved of it: reporting the copy removes both.
    let original = ctx.personal(&alice, "よくない").await;
    let saved = ctx.save(&bob, &original.stamp_ref.id).await;
    let copy_id = saved.stamp.stamp_ref.id.clone();
    let keeps = ctx.personal(&alice, "だいじょうぶ").await;
    ctx.stamp.handle_event(content_removed(SubjectType::SUBJECT_TYPE_STAMP, &copy_id, &moderator)).await.unwrap();
    let ids: Vec<Uuid> =
        [&original.stamp_ref.id, &copy_id, &keeps.stamp_ref.id].iter().map(|id| id.parse().unwrap()).collect();
    let left = stamp_refs(core, &ids).await.unwrap();
    assert_eq!(left.keys().copied().collect::<Vec<_>>(), vec![ids[2]], "only the unrelated stamp is left");
    // The picture itself is deleted too: it must not stay online or become a stamp again.
    assert!(!ctx.upload_exists(asset_of(&original)).await);
    assert!(ctx.upload_exists(asset_of(&keeps)).await);
    for user in [&alice, &bob] {
        let notices = ctx.notifications(user, "stamp.stamp.removed").await;
        assert_eq!(notices.len(), 1);
        assert!(notices[0]["body"]["ja"].as_str().unwrap().contains("権利侵害"));
    }

    // A stamp of a published pack: the pack stays, one stamp shorter.
    let pack = ctx.published(&alice, "ふたつ", 0, &[], 2).await;
    let id = id_of(&pack);
    ctx.stamp
        .handle_event(content_removed(SubjectType::SUBJECT_TYPE_STAMP, &pack.stamps[0].stamp_ref.id, &moderator))
        .await
        .unwrap();
    let after = ctx.get_pack(None, &id).await.unwrap();
    assert_eq!(stamp_names(&after.stamps), vec!["スタンプ2"]);
    assert_eq!(after.summary.stamp_count, 1);
    assert_eq!(after.summary.cover.url, pack.stamps[1].image.url, "the cover moves on to a stamp that still exists");
    assert_eq!(ctx.notifications(&alice, "stamp.stamp.removed").await.len(), 2);

    // Losing its last stamp takes a pack out of the store.
    ctx.stamp
        .handle_event(content_removed(SubjectType::SUBJECT_TYPE_STAMP, &pack.stamps[1].stamp_ref.id, &moderator))
        .await
        .unwrap();
    assert_eq!(ctx.get_pack(None, &id).await.unwrap_err().reason, "PACK_NOT_FOUND");
    assert_eq!(ctx.get_pack(Some(&alice), &id).await.unwrap().summary.status, PackStatus::PACK_STATUS_DRAFT);
    assert_eq!(ctx.search_changes(&id).await.last(), Some(&false));
    // A stamp that is already gone is not an error.
    ctx.stamp
        .handle_event(content_removed(SubjectType::SUBJECT_TYPE_STAMP, &pack.stamps[1].stamp_ref.id, &moderator))
        .await
        .unwrap();
}
