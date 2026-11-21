//! What engagement does when other domains speak: takedowns, erased
//! accounts, blocks, deleted uploads — and what it registers with the
//! shared registries (subject resolvers, the topic guard, the data export).
mod common;

use std::time::Duration;

use common::*;
use tokuly_proto::{
    common::{SubjectRef, SubjectType},
    engagement::{__buffa::oneof::engagement_event::Event as Pushed, *},
    media::AssetPurpose,
};
use uuid::Uuid;

async fn count(ctx: &Ctx, sql: &str, id: Uuid) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned())).bind(id).fetch_one(&ctx.app.core.db).await.unwrap()
}

fn save(subject: SubjectRef) -> AddBookmarkRequest {
    AddBookmarkRequest { subject: subject.into(), ..Default::default() }
}

fn report(subject: SubjectRef) -> CreateReportRequest {
    CreateReportRequest { subject: subject.into(), category: ReportCategory::REPORT_CATEGORY_SPAM.into(), ..Default::default() }
}

#[tokio::test]
async fn a_takedown_removes_a_comment_or_everything_on_a_subject() {
    let ctx = start().await;
    let (owner, bob, carol, moderator) =
        (ctx.app.user("Owner").await, ctx.app.user("Bob").await, ctx.app.user("Carol").await, ctx.app.user("Mod").await);
    let (video, kept) = (ctx.video(&owner, "消される動画"), ctx.video(&owner, "残る動画"));
    let top = ctx.comment(&bob, &video, "スレッド").await;
    let reply = ctx.reply(&carol, &top.id, "返信").await;
    let other = ctx.comment(&carol, &video, "べつのコメント").await;
    let on_kept = ctx.comment(&bob, &kept, "残る動画のコメント").await;
    ctx.react(&carol, comment_ref(&top.id), like()).await;
    ctx.react(&bob, video.subject_ref(), like()).await;
    let _: AddBookmarkResponse = ctx.call("BookmarkService", "AddBookmark", Some(&bob), &save(video.subject_ref())).await.unwrap();
    let _: AddBookmarkResponse = ctx.call("BookmarkService", "AddBookmark", Some(&bob), &save(kept.subject_ref())).await.unwrap();
    let _: CreateReportResponse = ctx.call("ReportService", "CreateReport", Some(&carol), &report(comment_ref(&top.id))).await.unwrap();
    let removal = |kind: i32, id: &str| serde_json::json!({ "subject_type": kind, "subject_id": id, "reason": "規約違反", "moderator_id": moderator.id });

    // A comment: gone with its replies; the report about it is answered.
    let mut topic = ctx.app.listen_topic(&video.topic()).await;
    ctx.deliver("admin.content.removed", Some(moderator.id), removal(10, &top.id)).await;
    assert_eq!(texts(&ctx.comments(None, &video).await.comments), ["べつのコメント"]);
    assert_eq!(ctx.videos.counter(video.id, "comment_count"), 1);
    assert_eq!(ctx.get_comment(Some(&carol), &reply.id).await.unwrap_err().reason, "COMMENT_NOT_FOUND");
    assert!(drain(&mut topic).await.iter().any(|event| matches!(event, Pushed::CommentRemoved(removed) if removed.removed_count == 2)));
    let reports: ListMyReportsResponse = ctx.call("ReportService", "ListMyReports", Some(&carol), &ListMyReportsRequest::default()).await.unwrap();
    assert_eq!(reports.reports[0].status, ReportStatus::REPORT_STATUS_ACTION_TAKEN);
    // Delivered twice, it changes nothing.
    ctx.deliver("admin.content.removed", Some(moderator.id), removal(10, &top.id)).await;
    assert_eq!(ctx.videos.counter(video.id, "comment_count"), 1);

    // Content of another domain: everything engagement kept about it goes.
    ctx.deliver("admin.content.removed", Some(moderator.id), removal(5, &video.id.to_string())).await;
    for table in ["comments", "reactions", "subject_stats", "bookmarks"] {
        let left = count(&ctx, &format!("SELECT count(*) FROM {table} WHERE subject_id = $1"), video.id).await;
        assert_eq!(left, 0, "{table}");
    }
    assert_eq!(ctx.get_comment(Some(&carol), &other.id).await.unwrap_err().reason, "COMMENT_NOT_FOUND");
    // The bookmark collection's size followed; the other video is untouched.
    let collections: ListCollectionsResponse =
        ctx.call("BookmarkService", "ListCollections", Some(&bob), &ListCollectionsRequest::default()).await.unwrap();
    assert_eq!(collections.collections[0].item_count, 1);
    assert_eq!(ctx.get_comment(None, &on_kept.id).await.unwrap().comment.text, "残る動画のコメント");
    assert_eq!(ctx.engagement_of(Some(&bob), &kept).await.bookmark_count, 1);
    ctx.deliver("admin.content.removed", Some(moderator.id), removal(5, &video.id.to_string())).await;
}

#[tokio::test]
async fn an_erased_account_leaves_nothing_behind() {
    let ctx = start().await;
    let (owner, gone, carol) = (ctx.app.user("Owner").await, ctx.app.user("Gone").await, ctx.app.user("Carol").await);
    let (video, theirs) = (ctx.video(&owner, "残る動画"), ctx.video(&gone, "消える人の動画"));

    // What the account wrote and did on other people's content...
    let their_thread = ctx.comment(&gone, &video, "消える人のスレッド").await;
    ctx.reply(&carol, &their_thread.id, "そこへの返信").await;
    let carols = ctx.comment(&carol, &video, "キャロルのスレッド").await;
    ctx.reply(&gone, &carols.id, "消える人の返信").await;
    ctx.react(&gone, video.subject_ref(), like()).await;
    ctx.react(&gone, comment_ref(&carols.id), like()).await;
    let _: AddBookmarkResponse = ctx.call("BookmarkService", "AddBookmark", Some(&gone), &save(video.subject_ref())).await.unwrap();
    let _: CreateReportResponse = ctx.call("ReportService", "CreateReport", Some(&gone), &report(video.subject_ref())).await.unwrap();
    ctx.set_rules(&gone, true, false, &["x"]).await;
    // ...and what other people left on the account's content.
    ctx.set_rules(&gone, false, false, &[]).await;
    ctx.comment(&carol, &theirs, "消える人の動画へのコメント").await;
    ctx.react(&carol, theirs.subject_ref(), emoji("🎉")).await;
    let _: AddBookmarkResponse = ctx.call("BookmarkService", "AddBookmark", Some(&carol), &save(theirs.subject_ref())).await.unwrap();
    let _: CreateReportResponse = ctx.call("ReportService", "CreateReport", Some(&carol), &report(theirs.subject_ref())).await.unwrap();
    let block = BlockCommenterRequest {
        target: Some(tokuly_proto::engagement::__buffa::oneof::block_commenter_request::Target::UserId(gone.id.to_string())),
        ..Default::default()
    };
    let _: BlockCommenterResponse = ctx.call("CommentModerationService", "BlockCommenter", Some(&carol), &block).await.unwrap();
    assert_eq!(ctx.videos.counter(video.id, "comment_count"), 4);

    ctx.deliver("identity.user.deleted", Some(gone.id), serde_json::json!({ "user_id": gone.id })).await;

    // Their comments are gone, with the replies below their thread; other people's stay.
    let listed = ctx.comments(None, &video).await;
    assert_eq!((texts(&listed.comments), listed.total_count), (vec!["キャロルのスレッド"], 1));
    assert_eq!(listed.comments[0].reply_count, 0);
    assert_eq!(listed.comments[0].like_count, 0);
    assert_eq!(ctx.videos.counter(video.id, "comment_count"), 1);
    // Their reactions and bookmarks no longer count.
    let engagement = ctx.engagement_of(None, &video).await;
    assert_eq!((engagement.reactions.like_count, engagement.bookmark_count), (0, 0));
    assert_eq!((ctx.videos.counter(video.id, "like_count"), ctx.videos.counter(video.id, "bookmark_count")), (0, 0));
    // Nothing is stored under their name any more.
    for (table, column) in [
        ("comments", "author_id"),
        ("reactions", "user_id"),
        ("bookmarks", "user_id"),
        ("bookmark_collections", "user_id"),
        ("reports", "reporter_id"),
        ("comment_settings", "user_id"),
        ("comment_blocks", "blocked_user_id"),
        ("comment_mentions", "user_id"),
    ] {
        assert_eq!(count(&ctx, &format!("SELECT count(*) FROM {table} WHERE {column} = $1"), gone.id).await, 0, "{table}");
    }
    // What hung off their content went with it; the report about it stays with the team, without the name.
    for table in ["comments", "reactions", "subject_stats", "bookmarks"] {
        assert_eq!(count(&ctx, &format!("SELECT count(*) FROM {table} WHERE subject_id = $1"), theirs.id).await, 0, "{table}");
    }
    assert_eq!(count(&ctx, "SELECT count(*) FROM reports WHERE subject_id = $1 AND subject_owner_id IS NULL", theirs.id).await, 1);
    let carols_collections: ListCollectionsResponse =
        ctx.call("BookmarkService", "ListCollections", Some(&carol), &ListCollectionsRequest::default()).await.unwrap();
    assert_eq!(carols_collections.collections[0].item_count, 0);

    // A second delivery finds nothing to do.
    ctx.deliver("identity.user.deleted", Some(gone.id), serde_json::json!({ "user_id": gone.id })).await;
    assert_eq!(ctx.videos.counter(video.id, "comment_count"), 1);
}

#[tokio::test]
async fn a_block_clears_the_blocked_users_comments_from_the_blockers_content() {
    let ctx = start().await;
    let (owner, bob, carol) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await, ctx.app.user("Carol").await);
    let (video, carols) = (ctx.video(&owner, "オーナーの動画"), ctx.video(&carol, "キャロルの動画"));
    let pinned = ctx.comment(&bob, &video, "ピンされてたコメント").await;
    let pin = PinCommentRequest { comment_id: pinned.id.clone(), ..Default::default() };
    let _: PinCommentResponse = ctx.call("CommentService", "PinComment", Some(&owner), &pin).await.unwrap();
    ctx.set_rules(&owner, true, false, &[]).await;
    ctx.comment(&bob, &video, "承認待ちだったコメント").await;
    ctx.set_rules(&owner, false, false, &[]).await;
    ctx.comment(&carol, &video, "キャロルのコメント").await;
    let elsewhere = ctx.comment(&bob, &carols, "よその動画へのコメント").await;

    ctx.block(&owner, &bob).await;

    // Nobody sees bob's comments on the owner's video any more — not just the owner.
    assert_eq!(texts(&ctx.comments(Some(&carol), &video).await.comments), ["キャロルのコメント"]);
    assert_eq!(ctx.videos.counter(video.id, "comment_count"), 1);
    // The review queue is clean, and the pin is gone.
    let queue: ListReceivedCommentsResponse = ctx
        .call("CommentModerationService", "ListReceivedComments", Some(&owner), &ListReceivedCommentsRequest::default())
        .await
        .unwrap();
    assert_eq!((queue.held_count, queue.comments.len()), (0, 1));
    assert_eq!(count(&ctx, "SELECT count(*) FROM comments WHERE subject_id = $1 AND pinned_at IS NOT NULL", video.id).await, 0);
    // What bob wrote elsewhere is nobody else's business.
    assert_eq!(ctx.get_comment(Some(&carol), &elsewhere.id).await.unwrap().comment.text, "よその動画へのコメント");
}

#[tokio::test]
async fn a_deleted_upload_takes_the_picture_out_of_comments() {
    let ctx = start().await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    let asset_id = ctx.image(&bob, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await;
    let with_text = CreateCommentRequest {
        subject: video.subject_ref().into(),
        text: "写真つき".into(),
        image_asset_id: asset_id.to_string(),
        ..Default::default()
    };
    let with_text: CreateCommentResponse = ctx.call("CommentService", "CreateComment", Some(&bob), &with_text).await.unwrap();
    let other_asset = ctx.image(&bob, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await;
    let only_image =
        CreateCommentRequest { subject: video.subject_ref().into(), image_asset_id: other_asset.to_string(), ..Default::default() };
    let only_image: CreateCommentResponse = ctx.call("CommentService", "CreateComment", Some(&bob), &only_image).await.unwrap();
    assert_eq!(ctx.videos.counter(video.id, "comment_count"), 2);

    // A comment with text keeps its text.
    ctx.deliver("media.asset.deleted", None, serde_json::json!({ "asset_id": asset_id, "owner_id": bob.id, "purpose": 8, "kind": 1 })).await;
    let kept = ctx.get_comment(None, &with_text.comment.id).await.unwrap();
    assert_eq!(kept.comment.text, "写真つき");
    assert!(kept.comment.image.as_option().is_none());
    // A comment that was only the picture goes.
    ctx.deliver("media.asset.deleted", None, serde_json::json!({ "asset_id": other_asset, "owner_id": bob.id, "purpose": 8, "kind": 1 })).await;
    assert_eq!(ctx.get_comment(None, &only_image.comment.id).await.unwrap_err().reason, "COMMENT_NOT_FOUND");
    assert_eq!(ctx.videos.counter(video.id, "comment_count"), 1);
}

#[tokio::test]
async fn only_those_who_see_a_subject_may_follow_its_topic() {
    let ctx = start().await;
    let (owner, friend, stranger) = (ctx.app.user("Owner").await, ctx.app.user("Friend").await, ctx.app.user("Stranger").await);
    let (public, private) = (ctx.video(&owner, "公開"), ctx.video(&owner, "限定"));
    ctx.videos.update(private.id, |subject| subject.audience = Audience::Only(vec![friend.id]));
    let guards = &ctx.app.core.topic_guards;

    assert!(guards.may_subscribe(None, &public.topic()).await.unwrap());
    assert!(guards.may_subscribe(Some(stranger.id), &public.topic()).await.unwrap());
    assert!(guards.may_subscribe(Some(friend.id), &private.topic()).await.unwrap());
    assert!(guards.may_subscribe(Some(owner.id), &private.topic()).await.unwrap());
    assert!(!guards.may_subscribe(Some(stranger.id), &private.topic()).await.unwrap());
    assert!(!guards.may_subscribe(None, &private.topic()).await.unwrap());
    // Unknown subjects, types nobody serves and malformed topics are closed.
    assert!(!guards.may_subscribe(None, &format!("subject:5:{}", Uuid::now_v7())).await.unwrap());
    assert!(!guards.may_subscribe(None, &format!("subject:15:{}", Uuid::now_v7())).await.unwrap());
    assert!(!guards.may_subscribe(None, "subject:5:not-a-uuid").await.unwrap());
    assert!(!guards.may_subscribe(None, "subject:nonsense").await.unwrap());
    // A block closes the topic like everything else about the subject.
    ctx.block(&owner, &stranger).await;
    assert!(!guards.may_subscribe(Some(stranger.id), &public.topic()).await.unwrap());
}

#[tokio::test]
async fn comments_and_users_are_subjects() {
    let ctx = start().await;
    let (owner, bob, carol) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await, ctx.app.user("Carol").await);
    let video = ctx.video(&owner, "v");
    let comment = ctx.comment(&bob, &video, &"ながいコメント".repeat(12)).await;
    let comment_id = comment.id.as_str();
    let subjects = &ctx.app.core.subjects;
    let kind = SubjectType::SUBJECT_TYPE_COMMENT;

    // A comment resolves to its author, an excerpt, and a link into its subject.
    let info = subjects.resolve(kind, comment_id, None).await.unwrap();
    assert_eq!((info.id.to_string().as_str(), info.owner_id), (comment_id, bob.id));
    assert_eq!(info.path, format!("{}?comment={comment_id}", video.path));
    assert!(info.title.chars().count() == 60 && info.title.ends_with('…'));
    assert!(info.interactions_enabled);
    assert_eq!(subjects.resolve(kind, "not-a-uuid", None).await.unwrap_err().reason, "SUBJECT_NOT_FOUND");
    assert_eq!(subjects.resolve(kind, &Uuid::now_v7().to_string(), None).await.unwrap_err().reason, "SUBJECT_NOT_FOUND");

    // It follows the visibility of the comment: hidden, it resolves for its author and the owner only, and takes no likes.
    let hide = HideCommentRequest { comment_id: comment.id.clone(), ..Default::default() };
    let _: HideCommentResponse = ctx.call("CommentModerationService", "HideComment", Some(&owner), &hide).await.unwrap();
    assert!(subjects.resolve(kind, comment_id, Some(carol.id)).await.is_err());
    assert!(subjects.resolve(kind, comment_id, None).await.is_err());
    assert!(!subjects.resolve(kind, comment_id, Some(bob.id)).await.unwrap().interactions_enabled);
    assert!(!subjects.resolve(kind, comment_id, Some(owner.id)).await.unwrap().interactions_enabled);
    // And the visibility of what it is on.
    let approve = ApproveCommentRequest { comment_id: comment.id.clone(), ..Default::default() };
    let _: ApproveCommentResponse = ctx.call("CommentModerationService", "ApproveComment", Some(&owner), &approve).await.unwrap();
    ctx.videos.update(video.id, |subject| subject.audience = Audience::Only(vec![carol.id]));
    assert!(subjects.resolve(kind, comment_id, Some(carol.id)).await.is_ok());
    assert!(subjects.resolve(kind, comment_id, None).await.is_err());

    // Users resolve by UUID or handle; they can be reported, saved and shared, not liked or commented on.
    let user = SubjectType::SUBJECT_TYPE_USER;
    let by_id = subjects.resolve(user, &bob.id.to_string(), None).await.unwrap();
    let by_handle = subjects.resolve(user, &bob.handle.to_uppercase(), Some(carol.id)).await.unwrap();
    assert_eq!((by_id.id, by_handle.id, by_id.owner_id), (bob.id, bob.id, bob.id));
    assert_eq!((by_id.title.as_str(), by_id.path.as_str()), ("Bob", format!("/u/{}", bob.handle).as_str()));
    assert!(!by_id.interactions_enabled);
    assert!(subjects.resolve(user, "no-such-handle", None).await.is_err());
    assert!(subjects.resolve(user, "!!", None).await.is_err());
    let profile = SubjectRef { r#type: user.into(), id: bob.id.to_string(), ..Default::default() };
    let like_request = SetReactionRequest { subject: profile.clone().into(), reaction: like().into(), ..Default::default() };
    assert_eq!(ctx.reason("ReactionService", "SetReaction", Some(&carol), &like_request).await, "INTERACTIONS_DISABLED");
    let comment_request = CreateCommentRequest { subject: profile.clone().into(), text: "やあ".into(), ..Default::default() };
    assert_eq!(ctx.reason("CommentService", "CreateComment", Some(&carol), &comment_request).await, "COMMENTS_DISABLED");
    let saved: AddBookmarkResponse = ctx.call("BookmarkService", "AddBookmark", Some(&carol), &save(profile)).await.unwrap();
    assert_eq!(saved.bookmark_count, 1);
    // An erased account is no subject any more.
    sqlx::query("UPDATE users SET deleted_at = now() WHERE id = $1").bind(bob.id).execute(&ctx.app.core.db).await.unwrap();
    assert!(subjects.resolve(user, &bob.id.to_string(), None).await.is_err());
}

#[tokio::test]
async fn counter_pushes_are_throttled_but_end_on_the_final_totals() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let video = ctx.video(&owner, "v");
    let (a, b, c, d) = (ctx.app.user("A").await, ctx.app.user("B").await, ctx.app.user("C").await, ctx.app.user("D").await);
    let mut topic = ctx.app.listen_topic(&video.topic()).await;
    // Four likes at the same moment.
    tokio::join!(
        ctx.react(&a, video.subject_ref(), like()),
        ctx.react(&b, video.subject_ref(), like()),
        ctx.react(&c, video.subject_ref(), like()),
        ctx.react(&d, video.subject_ref(), like()),
    );
    let likes: Vec<i64> = pushed(topic.drain(Duration::from_millis(2500)).await)
        .iter()
        .filter_map(|event| match event {
            Pushed::CountersChanged(counters) => Some(counters.reactions.like_count),
            _ => None,
        })
        .collect();
    // One push at once, one when the quiet second is over — not one per like.
    assert_eq!(likes.len(), 2, "{likes:?}");
    assert!(likes[0] >= 1);
    assert_eq!(likes[1], 4, "the final totals always arrive");
}

#[tokio::test]
async fn the_data_export_contains_what_a_user_left_here() {
    let ctx = start().await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    let comment = ctx.comment(&bob, &video, "エクスポートされるコメント").await;
    ctx.react(&bob, video.subject_ref(), emoji("🎉")).await;
    let _: AddBookmarkResponse = ctx.call("BookmarkService", "AddBookmark", Some(&bob), &save(video.subject_ref())).await.unwrap();
    let _: CreateReportResponse = ctx.call("ReportService", "CreateReport", Some(&bob), &report(video.subject_ref())).await.unwrap();
    ctx.set_rules(&bob, true, false, &["ことば"]).await;
    let block = BlockCommenterRequest {
        target: Some(tokuly_proto::engagement::__buffa::oneof::block_commenter_request::Target::UserId(owner.id.to_string())),
        ..Default::default()
    };
    let _: BlockCommenterResponse = ctx.call("CommentModerationService", "BlockCommenter", Some(&bob), &block).await.unwrap();

    let exporter = ctx.app.core.exporters.all().into_iter().find(|exporter| exporter.name() == "engagement").expect("the exporter is registered");
    let data = exporter.export(bob.id).await.unwrap();
    assert_eq!(data["comments"][0]["id"], comment.id.as_str());
    assert_eq!(data["comments"][0]["text"], "エクスポートされるコメント");
    assert_eq!(data["reactions"][0]["emoji"], "🎉");
    assert_eq!(data["collections"][0]["name"], "saved");
    assert_eq!(data["collections"][0]["bookmarks"][0]["subject_id"], serde_json::json!(video.id));
    assert_eq!(data["reports"][0]["subject_id"], serde_json::json!(video.id));
    assert_eq!(data["comment_settings"]["hold_all"], true);
    assert_eq!(data["comment_settings"]["blocked_words"][0], "ことば");
    assert_eq!(data["blocked_commenters"][0]["user_id"], serde_json::json!(owner.id));
    // Someone who did nothing here exports empty sections.
    let empty = exporter.export(owner.id).await.unwrap();
    assert!(empty["comments"].as_array().unwrap().is_empty() && empty["comment_settings"].is_null());
}
