//! Submitting emoji and reviewing them: what a submission is checked for,
//! the limit on waiting submissions, trusted submitters, changing and
//! withdrawing one's own emoji, and the moderators' queue.
mod common;

use common::*;
use tokuly_emoji::{EmojiConfig, catalog};
use tokuly_proto::{
    emoji::{__buffa::oneof::emoji_event, *},
    media::{AssetKind, AssetPurpose},
};
use tokuly_testkit::TestUser;

async fn profile(ctx: &Ctx, user: &TestUser) -> SubmitterProfile {
    let response: GetSubmitterProfileResponse = ctx
        .call("Submission", "GetSubmitterProfile", Some(user), &GetSubmitterProfileRequest::default())
        .await
        .unwrap();
    response.profile.into_option().unwrap()
}

async fn my_submissions(ctx: &Ctx, user: &TestUser, state: EmojiState) -> Vec<EmojiDetail> {
    let request = ListMySubmissionsRequest { state: state.into(), page: page(100, ""), ..Default::default() };
    let response: ListMySubmissionsResponse =
        ctx.call("Submission", "ListMySubmissions", Some(user), &request).await.unwrap();
    response.emojis
}

async fn check_name(ctx: &Ctx, user: &TestUser, name: &str) -> CheckNameResponse {
    let request = CheckNameRequest { name: name.to_owned(), ..Default::default() };
    ctx.call("Submission", "CheckName", Some(user), &request).await.unwrap()
}

async fn update(ctx: &Ctx, user: &TestUser, request: UpdateSubmissionRequest) -> Result<EmojiDetail, String> {
    match ctx.call::<_, UpdateSubmissionResponse>("Submission", "UpdateSubmission", Some(user), &request).await {
        Ok(response) => Ok(response.emoji.into_option().unwrap()),
        Err(error) => Err(error.reason),
    }
}

/// Runs a statement against the fixture upload `asset_id`.
async fn alter_upload(ctx: &Ctx, asset_id: &str, set: &str) {
    sqlx::query(sqlx::AssertSqlSafe(format!("UPDATE media_assets SET {set} WHERE id = $1::uuid")))
        .bind(asset_id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
}

#[tokio::test]
async fn anyone_signed_in_submits_an_emoji_and_waits_for_review() {
    let ctx = start().await;
    let (moderator, author) = (ctx.moderator().await, ctx.app.user("author").await);
    let category = unique("たべもの");
    ctx.create_category(&moderator, &category).await;
    let name = unique("purin");
    let request = SubmitEmojiRequest {
        name: name.to_uppercase(),
        category: category.to_uppercase(),
        aliases: strings(&["Pudding", "プリン"]),
        license: "じぶんでかいたよ".to_owned(),
        sensitive: true,
        ..ctx.submit_request(&author, &name).await
    };
    let asset_id = request.asset_id.clone();
    assert_eq!(ctx.reason("Submission", "SubmitEmoji", None, &request).await, "SIGN_IN_REQUIRED");
    let response: SubmitEmojiResponse = ctx.call("Submission", "SubmitEmoji", Some(&author), &request).await.unwrap();
    let submitted = response.emoji.into_option().unwrap();

    assert_eq!(submitted.state, EmojiState::EMOJI_STATE_PENDING);
    assert_eq!(submitted.emoji.name, name, "capitals are lowered");
    assert_eq!(submitted.emoji.category, category, "the category is found without regard to case");
    assert_eq!(submitted.emoji.aliases, strings(&["pudding", "プリン"]));
    assert!(submitted.emoji.sensitive && !submitted.emoji.local_only);
    assert!(!submitted.emoji.added_at.is_set());
    assert_eq!(
        (submitted.author.id.as_str(), submitted.license.as_str()),
        (author.id.to_string().as_str(), "じぶんでかいたよ")
    );
    assert!(submitted.can_edit);
    assert_eq!(ctx.holders(&asset_id).await, 1, "the emoji keeps its upload in use");

    // It is nowhere yet: not in the catalogue, not for other domains.
    assert!(named(&ctx.catalog().await.emojis, &name).is_none());
    assert!(!catalog::exists(&ctx.app.core, &name).await.unwrap());
    assert_eq!(ctx.emoji_events("emoji.emoji.submitted", &submitted.emoji.id).await.len(), 1);
    assert!(ctx.emoji_events("emoji.emoji.added", &submitted.emoji.id).await.is_empty());
    assert!(ctx.search_changes(&submitted.emoji.id).await.is_empty());

    // The author follows it.
    let mine = my_submissions(&ctx, &author, EmojiState::EMOJI_STATE_UNSPECIFIED).await;
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].emoji.id, submitted.emoji.id);
    assert_eq!(my_submissions(&ctx, &author, EmojiState::EMOJI_STATE_PENDING).await.len(), 1);
    assert!(my_submissions(&ctx, &author, EmojiState::EMOJI_STATE_APPROVED).await.is_empty());
    let me = profile(&ctx, &author).await;
    assert!(!me.trusted);
    assert_eq!((me.pending_count, me.approved_count, me.max_pending), (1, 0, 10));
    assert_eq!((me.max_image_bytes, me.min_image_edge, me.max_aliases), (1024 * 1024, 16, 16));
    assert_eq!(
        ctx.reason("Submission", "ListMySubmissions", None, &ListMySubmissionsRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );
    assert_eq!(
        ctx.reason("Submission", "GetSubmitterProfile", None, &GetSubmitterProfileRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );
}

#[tokio::test]
async fn a_submission_is_checked_before_it_is_accepted() {
    let ctx = start().await;
    let (moderator, author, other) = (ctx.moderator().await, ctx.app.user("author").await, ctx.app.user("other").await);
    let taken = unique("taken");
    ctx.add(&moderator, &taken).await;
    let good = ctx.submit_request(&author, &unique("good")).await;
    let refuse =
        async |request: SubmitEmojiRequest| ctx.reason("Submission", "SubmitEmoji", Some(&author), &request).await;

    for bad in ["", "a", "__", "blob-cat", "ねこ", "has space", &"a".repeat(33)] {
        assert_eq!(
            refuse(SubmitEmojiRequest { name: bad.to_owned(), ..good.clone() }).await,
            "NAME_INVALID",
            "{bad:?}"
        );
    }
    assert_eq!(refuse(SubmitEmojiRequest { name: "Tokuly".to_owned(), ..good.clone() }).await, "NAME_RESERVED");
    assert_eq!(refuse(SubmitEmojiRequest { name: "admin".to_owned(), ..good.clone() }).await, "NAME_RESERVED");
    assert_eq!(refuse(SubmitEmojiRequest { name: taken.to_uppercase(), ..good.clone() }).await, "NAME_TAKEN");

    assert_eq!(refuse(SubmitEmojiRequest { asset_id: "nope".to_owned(), ..good.clone() }).await, "INVALID_ID");
    assert_eq!(
        refuse(SubmitEmojiRequest { asset_id: uuid::Uuid::now_v7().to_string(), ..good.clone() }).await,
        "ASSET_NOT_FOUND"
    );
    assert_eq!(
        refuse(SubmitEmojiRequest { asset_id: ctx.picture(&other).await, ..good.clone() }).await,
        "ASSET_NOT_FOUND",
        "somebody else's upload"
    );
    let avatar = ctx.upload(&author, AssetPurpose::ASSET_PURPOSE_AVATAR, AssetKind::ASSET_KIND_IMAGE).await.to_string();
    assert_eq!(refuse(SubmitEmojiRequest { asset_id: avatar, ..good.clone() }).await, "ASSET_WRONG_PURPOSE");
    // A picture uploaded as a stamp, the purpose emoji borrowed before they
    // had their own, is still an emoji picture.
    // (Somebody else's submission, so the author's count below stays put.)
    let as_stamp = ctx.upload(&other, AssetPurpose::ASSET_PURPOSE_STAMP, AssetKind::ASSET_KIND_IMAGE).await.to_string();
    let from_stamp = SubmitEmojiRequest { name: unique("old"), asset_id: as_stamp.clone(), ..good.clone() };
    let old: SubmitEmojiResponse = ctx.call("Submission", "SubmitEmoji", Some(&other), &from_stamp).await.unwrap();
    assert_eq!(old.emoji.emoji.image.asset_id, as_stamp);
    let lottie = ctx.upload(&author, AssetPurpose::ASSET_PURPOSE_STAMP, AssetKind::ASSET_KIND_FILE).await.to_string();
    assert_eq!(refuse(SubmitEmojiRequest { asset_id: lottie, ..good.clone() }).await, "ASSET_NOT_IMAGE");
    let video = ctx.upload(&author, AssetPurpose::ASSET_PURPOSE_STAMP, AssetKind::ASSET_KIND_VIDEO).await.to_string();
    assert_eq!(refuse(SubmitEmojiRequest { asset_id: video, ..good.clone() }).await, "ASSET_NOT_IMAGE");

    let processing = ctx.picture(&author).await;
    alter_upload(&ctx, &processing, "status = 2").await;
    assert_eq!(refuse(SubmitEmojiRequest { asset_id: processing, ..good.clone() }).await, "ASSET_NOT_READY");
    let failed = ctx.picture(&author).await;
    alter_upload(&ctx, &failed, "status = 4").await;
    assert_eq!(refuse(SubmitEmojiRequest { asset_id: failed, ..good.clone() }).await, "ASSET_FAILED");
    let heavy = ctx.picture(&author).await;
    alter_upload(&ctx, &heavy, "size_bytes = 1024 * 1024 + 1").await;
    assert_eq!(refuse(SubmitEmojiRequest { asset_id: heavy, ..good.clone() }).await, "IMAGE_TOO_LARGE");
    let tiny = ctx.picture(&author).await;
    alter_upload(&ctx, &tiny, "image = image || '{\"width\": 12, \"height\": 9}'").await;
    assert_eq!(refuse(SubmitEmojiRequest { asset_id: tiny, ..good.clone() }).await, "IMAGE_TOO_SMALL");

    assert_eq!(
        refuse(SubmitEmojiRequest { category: unique("nocategory"), ..good.clone() }).await,
        "CATEGORY_NOT_FOUND"
    );
    assert_eq!(
        refuse(SubmitEmojiRequest { aliases: strings(&["two words"]), ..good.clone() }).await,
        "ALIASES_INVALID"
    );
    let many: Vec<String> = (0..17).map(|n| format!("alias{n}")).collect();
    assert_eq!(refuse(SubmitEmojiRequest { aliases: many, ..good.clone() }).await, "ALIASES_INVALID");
    assert_eq!(refuse(SubmitEmojiRequest { license: "x".repeat(1025), ..good.clone() }).await, "LICENSE_TOO_LONG");

    // Nothing of all that left a trace, and the good one goes through.
    assert_eq!(profile(&ctx, &author).await.pending_count, 0);
    let response: SubmitEmojiResponse = ctx.call("Submission", "SubmitEmoji", Some(&author), &good).await.unwrap();
    assert_eq!(response.emoji.state, EmojiState::EMOJI_STATE_PENDING);
    // A waiting emoji holds its name too.
    let again = ctx.submit_request(&other, &good.name).await;
    assert_eq!(ctx.reason("Submission", "SubmitEmoji", Some(&other), &again).await, "NAME_TAKEN");
}

#[tokio::test]
async fn names_can_be_checked_while_typing() {
    let ctx = start().await;
    let (moderator, user) = (ctx.moderator().await, ctx.app.user("user").await);
    let taken = unique("taken");
    ctx.add(&moderator, &taken).await;
    let free = unique("free");

    let available = check_name(&ctx, &user, &format!(":{}:", free.to_uppercase())).await;
    assert!(available.available && available.reason.is_empty());
    assert_eq!(available.name, free, "the name comes back as it would be stored");
    let clash = check_name(&ctx, &user, &taken).await;
    assert_eq!((clash.available, clash.reason.as_str(), clash.name.as_str()), (false, "NAME_TAKEN", taken.as_str()));
    let invalid = check_name(&ctx, &user, "だめ").await;
    assert_eq!((invalid.available, invalid.reason.as_str(), invalid.name.as_str()), (false, "NAME_INVALID", ""));
    let reserved = check_name(&ctx, &user, "Official").await;
    assert_eq!(
        (reserved.available, reserved.reason.as_str(), reserved.name.as_str()),
        (false, "NAME_RESERVED", "official")
    );
    // Reserved names are reserved for moderators.
    assert!(check_name(&ctx, &moderator, "official").await.available);
    assert_eq!(ctx.reason("Submission", "CheckName", None, &CheckNameRequest::default()).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn only_so_many_submissions_wait_at_once() {
    let ctx = start_with(EmojiConfig { max_pending: 2, ..Default::default() }).await;
    let (moderator, author) = (ctx.moderator().await, ctx.app.user("author").await);
    let first = ctx.submit(&author, &unique("first")).await;
    let second = ctx.submit(&author, &unique("second")).await;
    let third = ctx.submit_request(&author, &unique("third")).await;
    assert_eq!(ctx.reason("Submission", "SubmitEmoji", Some(&author), &third).await, "TOO_MANY_PENDING");
    assert_eq!(profile(&ctx, &author).await.pending_count, 2);

    // A decision makes room, whichever way it goes.
    ctx.approve(&moderator, &first.emoji.id).await;
    let third: SubmitEmojiResponse = ctx.call("Submission", "SubmitEmoji", Some(&author), &third).await.unwrap();
    let fourth = ctx.submit_request(&author, &unique("fourth")).await;
    assert_eq!(ctx.reason("Submission", "SubmitEmoji", Some(&author), &fourth).await, "TOO_MANY_PENDING");
    ctx.reject(&moderator, &second.emoji.id, "ちょっとちがうかな").await;
    let _: SubmitEmojiResponse = ctx.call("Submission", "SubmitEmoji", Some(&author), &fourth).await.unwrap();

    // Submitting a rejected emoji again counts like a new submission.
    let resubmit = UpdateSubmissionRequest { emoji_id: second.emoji.id.clone(), ..Default::default() };
    assert_eq!(update(&ctx, &author, resubmit.clone()).await.unwrap_err(), "TOO_MANY_PENDING");
    // So does withdrawing make room.
    let withdraw = WithdrawSubmissionRequest { emoji_id: third.emoji.emoji.id.clone(), ..Default::default() };
    let _: WithdrawSubmissionResponse =
        ctx.call("Submission", "WithdrawSubmission", Some(&author), &withdraw).await.unwrap();
    assert_eq!(update(&ctx, &author, resubmit).await.unwrap().state, EmojiState::EMOJI_STATE_PENDING);
    let me = profile(&ctx, &author).await;
    assert_eq!((me.pending_count, me.approved_count, me.max_pending), (2, 1, 2));
}

#[tokio::test]
async fn trusted_submitters_and_moderators_skip_the_review() {
    let reserved = unique("mascot");
    let ctx = start_with(EmojiConfig { reserved_names: vec![reserved.clone()], ..Default::default() }).await;
    let (moderator, author) = (ctx.moderator().await, ctx.app.user("author").await);

    let trust = SetSubmitterTrustedRequest { user_id: author.id.to_string(), trusted: true, ..Default::default() };
    assert_eq!(ctx.reason("Admin", "SetSubmitterTrusted", Some(&author), &trust).await, "FORBIDDEN");
    let nobody =
        SetSubmitterTrustedRequest { user_id: uuid::Uuid::now_v7().to_string(), trusted: true, ..Default::default() };
    assert_eq!(ctx.reason("Admin", "SetSubmitterTrusted", Some(&moderator), &nobody).await, "USER_NOT_FOUND");
    ctx.trust(&moderator, &author, true).await;
    ctx.trust(&moderator, &author, true).await;
    assert_eq!(ctx.notification_kinds(&author).await, ["emoji.trusted"], "told once, however often the flag is set");
    let changes = ctx.app.events_by("emoji.submitter.trust_changed", moderator.id).await;
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].data["user_id"], serde_json::json!(author.id));
    assert_eq!(changes[0].data["trusted"], true);
    assert!(profile(&ctx, &author).await.trusted);

    // No queue for them: the emoji is usable at once.
    let name = unique("trusted");
    let emoji = ctx.submit(&author, &name).await;
    assert_eq!(emoji.state, EmojiState::EMOJI_STATE_APPROVED);
    assert!(emoji.emoji.added_at.is_set() && !emoji.can_edit);
    assert!(catalog::exists(&ctx.app.core, &name).await.unwrap());
    let added = ctx.emoji_events("emoji.emoji.added", &emoji.emoji.id).await;
    assert_eq!(added.len(), 1);
    assert_eq!(added[0]["via"], "trusted");
    assert!(ctx.emoji_events("emoji.emoji.submitted", &emoji.emoji.id).await.is_empty());
    assert_eq!(ctx.notification_kinds(&author).await, ["emoji.trusted"], "nobody approved it, so nobody says so");
    let history = ctx.get(Some(&moderator), &emoji.emoji.id).await.unwrap().history;
    let actions: Vec<_> = history.iter().map(|record| record.action.as_known().unwrap()).collect();
    assert_eq!(actions, [ReviewAction::REVIEW_ACTION_APPROVED, ReviewAction::REVIEW_ACTION_SUBMITTED]);
    assert!(!history[0].actor.is_set(), "approved by the rule, not by a person");

    // What is in the catalogue is no longer the author's to change or take
    // back, trusted or not: others may want to report it.
    let withdraw = WithdrawSubmissionRequest { emoji_id: emoji.emoji.id.clone(), ..Default::default() };
    assert_eq!(
        ctx.reason("Submission", "WithdrawSubmission", Some(&author), &withdraw).await,
        "EMOJI_NOT_WITHDRAWABLE"
    );
    let change = UpdateSubmissionRequest {
        emoji_id: emoji.emoji.id.clone(),
        name: Some(unique("renamed")),
        ..Default::default()
    };
    assert_eq!(update(&ctx, &author, change).await.unwrap_err(), "EMOJI_NOT_EDITABLE");

    // Trust does not unlock reserved names; being a moderator does.
    let request = ctx.submit_request(&author, &reserved).await;
    assert_eq!(ctx.reason("Submission", "SubmitEmoji", Some(&author), &request).await, "NAME_RESERVED");
    let by_moderator = ctx.submit(&moderator, &reserved).await;
    assert_eq!(by_moderator.state, EmojiState::EMOJI_STATE_APPROVED);
    assert!(profile(&ctx, &moderator).await.trusted);

    let listed: ListTrustedSubmittersResponse = ctx
        .call("Admin", "ListTrustedSubmitters", Some(&moderator), &ListTrustedSubmittersRequest::default())
        .await
        .unwrap();
    let entry = listed.submitters.iter().find(|s| s.user.id == author.id.to_string()).expect("the author is listed");
    assert_eq!(entry.trusted_by.id, moderator.id.to_string());
    assert_eq!(entry.approved_count, 1);
    assert!(entry.trusted_at.is_set());

    // Without the flag, back to the queue.
    ctx.trust(&moderator, &author, false).await;
    assert_eq!(ctx.submit(&author, &unique("again")).await.state, EmojiState::EMOJI_STATE_PENDING);
    let listed: ListTrustedSubmittersResponse = ctx
        .call("Admin", "ListTrustedSubmitters", Some(&moderator), &ListTrustedSubmittersRequest::default())
        .await
        .unwrap();
    assert!(listed.submitters.iter().all(|s| s.user.id != author.id.to_string()));
    assert_eq!(ctx.app.events_by("emoji.submitter.trust_changed", moderator.id).await.len(), 2);
}

#[tokio::test]
async fn authors_change_and_withdraw_what_is_not_approved() {
    let ctx = start().await;
    let (moderator, author, stranger) =
        (ctx.moderator().await, ctx.app.user("author").await, ctx.app.user("stranger").await);
    let category = unique("かお");
    ctx.create_category(&moderator, &category).await;
    let request = ctx.submit_request(&author, &unique("draft")).await;
    let first_picture = request.asset_id.clone();
    let response: SubmitEmojiResponse = ctx.call("Submission", "SubmitEmoji", Some(&author), &request).await.unwrap();
    let id = response.emoji.emoji.id.clone();

    let renamed = unique("better");
    let second_picture = ctx.animation(&author).await;
    let change = UpdateSubmissionRequest {
        emoji_id: id.clone(),
        name: Some(renamed.clone()),
        asset_id: Some(second_picture.clone()),
        category: Some(category.clone()),
        aliases: Some(AliasList { aliases: strings(&["smile", "えがお"]), ..Default::default() }).into(),
        license: Some("CC0".to_owned()),
        sensitive: Some(true),
        ..Default::default()
    };
    assert_eq!(update(&ctx, &stranger, change.clone()).await.unwrap_err(), "EMOJI_NOT_FOUND");
    assert_eq!(ctx.reason("Submission", "UpdateSubmission", None, &change).await, "SIGN_IN_REQUIRED");
    let changed = update(&ctx, &author, change).await.unwrap();
    assert_eq!(changed.state, EmojiState::EMOJI_STATE_PENDING);
    assert_eq!((changed.emoji.name.as_str(), changed.emoji.category.as_str()), (renamed.as_str(), category.as_str()));
    assert_eq!(changed.emoji.aliases, strings(&["smile", "えがお"]));
    assert!(changed.emoji.animated && changed.emoji.sensitive);
    assert_eq!(changed.license, "CC0");
    assert_eq!(
        (ctx.holders(&first_picture).await, ctx.holders(&second_picture).await),
        (0, 1),
        "the old upload is released"
    );

    // Fields that are not set stay; an empty category clears it.
    let partial = UpdateSubmissionRequest { emoji_id: id.clone(), category: Some(String::new()), ..Default::default() };
    let partial = update(&ctx, &author, partial).await.unwrap();
    assert_eq!(
        (partial.emoji.category.as_str(), partial.emoji.name.as_str(), partial.license.as_str()),
        ("", renamed.as_str(), "CC0")
    );
    assert_eq!(partial.emoji.aliases.len(), 2);

    // The same checks as a submission apply.
    let taken = unique("taken");
    ctx.add(&moderator, &taken).await;
    let cases = [
        (UpdateSubmissionRequest { name: Some(taken.clone()), ..Default::default() }, "NAME_TAKEN"),
        (UpdateSubmissionRequest { name: Some("x".to_owned()), ..Default::default() }, "NAME_INVALID"),
        (UpdateSubmissionRequest { name: Some("staff".to_owned()), ..Default::default() }, "NAME_RESERVED"),
        (UpdateSubmissionRequest { category: Some(unique("nocategory")), ..Default::default() }, "CATEGORY_NOT_FOUND"),
        (
            UpdateSubmissionRequest { asset_id: Some(ctx.picture(&stranger).await), ..Default::default() },
            "ASSET_NOT_FOUND",
        ),
        (UpdateSubmissionRequest { license: Some("x".repeat(1025)), ..Default::default() }, "LICENSE_TOO_LONG"),
        (
            UpdateSubmissionRequest {
                aliases: Some(AliasList { aliases: strings(&["a b"]), ..Default::default() }).into(),
                ..Default::default()
            },
            "ALIASES_INVALID",
        ),
    ];
    for (request, reason) in cases {
        let request = UpdateSubmissionRequest { emoji_id: id.clone(), ..request };
        assert_eq!(update(&ctx, &author, request).await.unwrap_err(), reason);
    }
    let unknown = UpdateSubmissionRequest { emoji_id: uuid::Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(update(&ctx, &author, unknown).await.unwrap_err(), "EMOJI_NOT_FOUND");

    // Once approved it belongs to everyone who uses it.
    ctx.approve(&moderator, &id).await;
    let late = UpdateSubmissionRequest { emoji_id: id.clone(), license: Some("MIT".to_owned()), ..Default::default() };
    assert_eq!(update(&ctx, &author, late).await.unwrap_err(), "EMOJI_NOT_EDITABLE");
    let withdraw = WithdrawSubmissionRequest { emoji_id: id.clone(), ..Default::default() };
    assert_eq!(
        ctx.reason("Submission", "WithdrawSubmission", Some(&author), &withdraw).await,
        "EMOJI_NOT_WITHDRAWABLE"
    );
    assert!(catalog::exists(&ctx.app.core, &renamed).await.unwrap());

    // What still waits can be taken back, by its author only.
    let request = ctx.submit_request(&author, &unique("oops")).await;
    let picture = request.asset_id.clone();
    let response: SubmitEmojiResponse = ctx.call("Submission", "SubmitEmoji", Some(&author), &request).await.unwrap();
    let withdraw = WithdrawSubmissionRequest { emoji_id: response.emoji.emoji.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Submission", "WithdrawSubmission", Some(&stranger), &withdraw).await, "EMOJI_NOT_FOUND");
    assert_eq!(ctx.reason("Submission", "WithdrawSubmission", None, &withdraw).await, "SIGN_IN_REQUIRED");
    let _: WithdrawSubmissionResponse =
        ctx.call("Submission", "WithdrawSubmission", Some(&author), &withdraw).await.unwrap();
    assert_eq!(ctx.get(Some(&author), &withdraw.emoji_id).await.unwrap_err().reason, "EMOJI_NOT_FOUND");
    assert_eq!(ctx.holders(&picture).await, 0);
    assert_eq!(ctx.reason("Submission", "WithdrawSubmission", Some(&author), &withdraw).await, "EMOJI_NOT_FOUND");
    assert!(check_name(&ctx, &stranger, &request.name).await.available, "its name is free again");
}

#[tokio::test]
async fn moderators_work_through_the_review_queue() {
    let ctx = start().await;
    let (moderator, author) = (ctx.moderator().await, ctx.app.user("author").await);
    let older = ctx.submit(&author, &unique("older")).await;
    let newer = ctx.submit(&author, &unique("newer")).await;

    let request = ListReviewQueueRequest { page: page(100, ""), ..Default::default() };
    assert_eq!(ctx.reason("Admin", "ListReviewQueue", Some(&author), &request).await, "FORBIDDEN");
    assert_eq!(ctx.reason("Admin", "ListReviewQueue", None, &request).await, "SIGN_IN_REQUIRED");
    let queue: ListReviewQueueResponse =
        ctx.call("Admin", "ListReviewQueue", Some(&moderator), &request).await.unwrap();
    assert!(queue.pending_count >= 2);
    let ids: Vec<&str> = queue.emojis.iter().map(|detail| detail.emoji.id.as_str()).collect();
    let (first, second) = (
        ids.iter().position(|id| *id == older.emoji.id).expect("the older submission is in the queue"),
        ids.iter().position(|id| *id == newer.emoji.id).expect("the newer submission is in the queue"),
    );
    assert!(first < second, "the oldest submission comes first");
    let item = &queue.emojis[first];
    assert_eq!(item.author.id, author.id.to_string());
    assert_eq!(item.history.len(), 1, "moderators see what happened so far");
    // The queue pages.
    let small: ListReviewQueueResponse = ctx
        .call(
            "Admin",
            "ListReviewQueue",
            Some(&moderator),
            &ListReviewQueueRequest { page: page(1, ""), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(small.emojis.len(), 1);
    assert!(small.page.has_more && !small.page.next_cursor.is_empty());

    // Approval: into the catalogue, and the author hears about it.
    let mut inbox = ctx.app.listen_user(author.id).await;
    let approve = ApproveEmojiRequest { emoji_id: older.emoji.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Admin", "ApproveEmoji", Some(&author), &approve).await, "FORBIDDEN");
    let approved = ctx.approve(&moderator, &older.emoji.id).await;
    assert_eq!(approved.state, EmojiState::EMOJI_STATE_APPROVED);
    assert!(approved.reviewed_at.is_set() && approved.emoji.added_at.is_set());
    let actions: Vec<_> = approved.history.iter().map(|record| record.action.as_known().unwrap()).collect();
    assert_eq!(actions, [ReviewAction::REVIEW_ACTION_APPROVED, ReviewAction::REVIEW_ACTION_SUBMITTED]);
    assert_eq!(approved.history[0].actor.id, moderator.id.to_string());
    assert!(catalog::exists(&ctx.app.core, &older.emoji.name).await.unwrap());

    let pushed = next_matching(&mut inbox, |frame| match emoji_event(frame)? {
        emoji_event::Event::SubmissionUpdated(detail) if detail.emoji.id == older.emoji.id => Some(detail.state),
        _ => None,
    })
    .await;
    assert_eq!(pushed, Some(EmojiState::EMOJI_STATE_APPROVED.into()), "the author's open screen follows the decision");

    let notices = ctx.notifications(&author).await;
    assert_eq!(notices.len(), 1);
    let notice = &notices[0];
    assert_eq!(notice["kind"], "emoji.approved");
    assert_eq!(notice["path"], format!("/emoji/{}", older.emoji.name));
    assert!(notice["actor_id"].is_null(), "authors are not told which moderator decided");
    assert!(notice["title"]["ja"].as_str().unwrap().contains("承認"));
    assert!(notice["body"]["en"].as_str().unwrap().contains(&format!(":{}:", older.emoji.name)));
    assert_eq!(notice["subject"][1], serde_json::json!(older.emoji.id));

    let added = ctx.emoji_events("emoji.emoji.added", &older.emoji.id).await;
    assert_eq!((added.len(), added[0]["via"].as_str()), (1, Some("review")));
    let indexed = ctx.search_changes(&older.emoji.id).await;
    let doc = indexed.last().unwrap().as_ref().expect("an approved emoji is searchable");
    assert_eq!(
        (doc["title"].as_str(), doc["path"].as_str()),
        (Some(older.emoji.name.as_str()), Some(notice["path"].as_str().unwrap()))
    );
    assert!(
        doc["owner_id"].is_null(),
        "an emoji in the catalogue is everyone's: search must not hide it with its author"
    );
    let activity = ctx.app.events_by("activity.recorded", author.id).await;
    assert_eq!(activity.len(), 1);
    assert_eq!(
        (activity[0].data["kind"].as_str(), activity[0].data["audience"].as_str()),
        (Some("emoji.added"), Some("public"))
    );

    // Decisions are made once.
    assert_eq!(ctx.reason("Admin", "ApproveEmoji", Some(&moderator), &approve).await, "EMOJI_NOT_REVIEWABLE");
    let reject = RejectEmojiRequest {
        emoji_id: older.emoji.id.clone(),
        reason: "やっぱりだめ".to_owned(),
        ..Default::default()
    };
    assert_eq!(ctx.reason("Admin", "RejectEmoji", Some(&moderator), &reject).await, "EMOJI_NOT_REVIEWABLE");
    let unknown = ApproveEmojiRequest { emoji_id: uuid::Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(ctx.reason("Admin", "ApproveEmoji", Some(&moderator), &unknown).await, "EMOJI_NOT_FOUND");
    // The queue also takes names.
    let by_name = ApproveEmojiRequest { emoji_id: format!(":{}:", newer.emoji.name), ..Default::default() };
    let _: ApproveEmojiResponse = ctx.call("Admin", "ApproveEmoji", Some(&moderator), &by_name).await.unwrap();
    assert_eq!(my_submissions(&ctx, &author, EmojiState::EMOJI_STATE_APPROVED).await.len(), 2);
}

#[tokio::test]
async fn a_rejected_emoji_gives_its_name_back_and_can_be_submitted_again() {
    let ctx = start().await;
    let (moderator, author, rival) = (ctx.moderator().await, ctx.app.user("author").await, ctx.app.user("rival").await);
    let name = unique("contested");
    let emoji = ctx.submit(&author, &name).await;
    let id = emoji.emoji.id.clone();

    for reason in ["", "   ", &"x".repeat(501)] {
        let request = RejectEmojiRequest { emoji_id: id.clone(), reason: reason.to_owned(), ..Default::default() };
        assert_eq!(ctx.reason("Admin", "RejectEmoji", Some(&moderator), &request).await, "REASON_INVALID");
    }
    let reject =
        RejectEmojiRequest {
            emoji_id: id.clone(), reason: "権利がわからないよ".to_owned(), ..Default::default()
        };
    assert_eq!(ctx.reason("Admin", "RejectEmoji", Some(&author), &reject).await, "FORBIDDEN");
    let rejected = ctx.reject(&moderator, &id, "権利がわからないよ").await;
    assert_eq!(rejected.state, EmojiState::EMOJI_STATE_REJECTED);
    assert_eq!(rejected.review_note, "権利がわからないよ");

    // The author sees why; nobody else sees it at all.
    let mine = ctx.get(Some(&author), &id).await.unwrap();
    assert_eq!(
        (mine.state, mine.review_note.as_str()),
        (EmojiState::EMOJI_STATE_REJECTED.into(), "権利がわからないよ")
    );
    assert!(mine.can_edit && mine.reviewed_at.is_set());
    assert_eq!(ctx.get(Some(&rival), &id).await.unwrap_err().reason, "EMOJI_NOT_FOUND");
    let notices = ctx.notifications(&author).await;
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0]["kind"], "emoji.rejected");
    assert!(notices[0]["body"]["ja"].as_str().unwrap().contains("権利がわからないよ"));
    let decisions = ctx.emoji_events("emoji.emoji.rejected", &id).await;
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0]["reason"], "権利がわからないよ");
    assert_eq!(decisions[0]["moderator_id"], serde_json::json!(moderator.id));

    // The name is free: somebody else takes it.
    assert!(check_name(&ctx, &rival, &name).await.available);
    let taken = ctx.submit(&rival, &name).await;
    assert_eq!(
        ctx.get(Some(&moderator), &name).await.unwrap().emoji.id,
        taken.emoji.id,
        "a name leads to the emoji holding it"
    );

    // Sending the rejected one back as it is no longer works; under a new name it does.
    let unchanged = UpdateSubmissionRequest { emoji_id: id.clone(), ..Default::default() };
    assert_eq!(update(&ctx, &author, unchanged).await.unwrap_err(), "NAME_TAKEN");
    let approve = ApproveEmojiRequest { emoji_id: id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("Admin", "ApproveEmoji", Some(&moderator), &approve).await, "NAME_TAKEN");
    let renamed = unique("contested");
    let again = update(
        &ctx,
        &author,
        UpdateSubmissionRequest { emoji_id: id.clone(), name: Some(renamed.clone()), ..Default::default() },
    )
    .await
    .unwrap();
    assert_eq!((again.state, again.review_note.as_str()), (EmojiState::EMOJI_STATE_PENDING.into(), ""));
    assert_eq!(ctx.emoji_events("emoji.emoji.submitted", &id).await.len(), 2);
    let history = ctx.get(Some(&moderator), &id).await.unwrap().history;
    let actions: Vec<_> = history.iter().map(|record| record.action.as_known().unwrap()).collect();
    assert_eq!(
        actions,
        [
            ReviewAction::REVIEW_ACTION_SUBMITTED,
            ReviewAction::REVIEW_ACTION_REJECTED,
            ReviewAction::REVIEW_ACTION_SUBMITTED
        ]
    );
    assert_eq!(history[1].note, "権利がわからないよ");

    // A moderator who changes their mind approves a rejected emoji directly.
    ctx.reject(&moderator, &id, "もういちど").await;
    let approved = ctx.approve(&moderator, &id).await;
    assert_eq!(approved.state, EmojiState::EMOJI_STATE_APPROVED);
    assert!(catalog::exists(&ctx.app.core, &renamed).await.unwrap());
    assert_eq!(ctx.notification_kinds(&author).await, ["emoji.rejected", "emoji.rejected", "emoji.approved"]);
}
