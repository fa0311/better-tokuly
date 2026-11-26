//! `CommentService`: writing, reading, editing and deleting comments.
mod common;

use common::*;
use tokuly_engagement::EngagementConfig;
use tokuly_proto::{
    common::{SubjectRef, SubjectType},
    engagement::{__buffa::oneof::engagement_event::Event as Pushed, *},
    media::AssetPurpose,
};
use uuid::Uuid;

const SVC: &str = "CommentService";

fn create(subject: &FakeSubject, text: &str) -> CreateCommentRequest {
    CreateCommentRequest { subject: subject.subject_ref().into(), text: text.to_owned(), ..Default::default() }
}

#[tokio::test]
async fn a_comment_is_published_counted_announced_and_pushed() {
    let ctx = start().await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "夏の思い出");
    let mut topic = ctx.app.listen_topic(&video.topic()).await;

    // The short ID names the same subject; the response carries the UUID.
    let request = CreateCommentRequest {
        subject: video.short_ref().into(),
        text: "  最高だった!  ".into(),
        timestamp_seconds: Some(83),
        ..Default::default()
    };
    let created: CreateCommentResponse = ctx.call(SVC, "CreateComment", Some(&bob), &request).await.unwrap();
    let comment = &created.comment;
    assert_eq!(comment.text, "最高だった!");
    assert_eq!(comment.subject.id, video.id.to_string());
    assert_eq!(comment.status, CommentStatus::COMMENT_STATUS_PUBLISHED);
    assert_eq!(comment.timestamp_seconds, Some(83));
    assert_eq!(comment.author.user.id, bob.id.to_string());
    assert!(comment.author.channel.as_option().is_none());
    assert!(comment.parent_id.is_empty() && !comment.edited && !comment.pinned && !comment.hearted);
    let state = &comment.viewer;
    assert!(state.is_author && state.can_edit && state.can_delete && !state.can_moderate);

    // Everyone sees it, guests included.
    let listed = ctx.comments(None, &video).await;
    assert_eq!(ids(&listed.comments), [comment.id.as_str()]);
    assert!(listed.comments[0].viewer.as_option().is_none(), "guests have no viewer state");
    assert_eq!((listed.total_count, listed.comments_enabled, listed.viewer_can_moderate), (1, true, false));
    let as_owner = ctx.comments(Some(&owner), &video).await;
    assert!(as_owner.viewer_can_moderate);
    assert!(
        as_owner.comments[0].viewer.can_moderate
            && as_owner.comments[0].viewer.can_delete
            && !as_owner.comments[0].viewer.can_edit
    );

    // The owning domain heard about its counter.
    assert_eq!(ctx.videos.counter(video.id, "comment_count"), 1);
    // Other domains can follow along.
    let published = ctx.events_about("engagement.comment.published", video.id).await;
    assert_eq!(published.len(), 1);
    assert_eq!(published[0]["comment_id"], comment.id.as_str());
    assert_eq!(published[0]["author_id"], serde_json::json!(bob.id));
    assert_eq!(published[0]["subject_type"], 5);

    // The owner is told, by the commenter.
    let notices = ctx.notifications(owner.id, "engagement.comment").await;
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0]["actor_id"], serde_json::json!(bob.id));
    assert_eq!(notices[0]["title"]["ja"], "Bobが「夏の思い出」にコメントしたよ!");
    assert_eq!(notices[0]["body"]["en"], "最高だった!");
    assert_eq!(notices[0]["path"], format!("{}?comment={}", video.path, comment.id));
    assert_eq!(notices[0]["subject"], serde_json::json!([5, video.id]));
    assert!(ctx.notification_kinds(bob.id).await.is_empty(), "nobody is told about their own comment");

    // Watchers of the subject get the comment and the new totals.
    let events = drain(&mut topic).await;
    let added = events.iter().find_map(|event| match event {
        Pushed::CommentAdded(added) => Some(added),
        _ => None,
    });
    let added = added.expect("a CommentAdded push");
    assert_eq!(added.comment_id, comment.id);
    assert_eq!(added.comment.text, "最高だった!");
    assert!(added.comment.viewer.as_option().is_none());
    let counters = events.iter().find_map(|event| match event {
        Pushed::CountersChanged(counters) => Some(counters),
        _ => None,
    });
    assert_eq!(counters.expect("a CountersChanged push").comment_count, 1);
}

#[tokio::test]
async fn comments_are_validated() {
    let ctx = start().await;
    let (owner, bob, carol) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await, ctx.app.user("Carol").await);
    let video = ctx.video(&owner, "v");
    let reason = |request: CreateCommentRequest| {
        let ctx = &ctx;
        let bob = &bob;
        async move { ctx.reason(SVC, "CreateComment", Some(bob), &request).await }
    };

    assert_eq!(ctx.reason(SVC, "CreateComment", None, &create(&video, "hi")).await, "SIGN_IN_REQUIRED");
    assert_eq!(reason(create(&video, "   ")).await, "COMMENT_EMPTY");
    assert_eq!(reason(create(&video, &"あ".repeat(1001))).await, "COMMENT_TOO_LONG");
    assert_eq!(
        reason(CreateCommentRequest { timestamp_seconds: Some(-1), ..create(&video, "t") }).await,
        "TIMESTAMP_INVALID"
    );
    assert_eq!(reason(CreateCommentRequest { parent_id: "nope".into(), ..create(&video, "t") }).await, "INVALID_ID");
    assert_eq!(
        reason(CreateCommentRequest { parent_id: Uuid::now_v7().to_string(), ..create(&video, "t") }).await,
        "COMMENT_NOT_FOUND"
    );

    // Subjects: unset, unknown to every domain, missing, hidden, closed.
    assert_eq!(reason(CreateCommentRequest { text: "t".into(), ..Default::default() }).await, "UNSUPPORTED_SUBJECT");
    let live = SubjectRef {
        r#type: SubjectType::SUBJECT_TYPE_LIVE_STREAM.into(),
        id: video.id.to_string(),
        ..Default::default()
    };
    assert_eq!(
        reason(CreateCommentRequest { subject: live.into(), text: "t".into(), ..Default::default() }).await,
        "UNSUPPORTED_SUBJECT"
    );
    let missing = SubjectRef {
        r#type: SubjectType::SUBJECT_TYPE_VIDEO.into(),
        id: Uuid::now_v7().to_string(),
        ..Default::default()
    };
    assert_eq!(
        reason(CreateCommentRequest { subject: missing.into(), text: "t".into(), ..Default::default() }).await,
        "SUBJECT_NOT_FOUND"
    );
    let private = ctx.video(&owner, "private");
    ctx.videos.update(private.id, |subject| subject.audience = Audience::Only(vec![carol.id]));
    assert_eq!(reason(create(&private, "t")).await, "SUBJECT_NOT_FOUND");
    assert!(
        ctx.call::<_, CreateCommentResponse>(SVC, "CreateComment", Some(&carol), &create(&private, "t")).await.is_ok()
    );
    let closed = ctx.video(&owner, "closed");
    ctx.videos.update(closed.id, |subject| subject.interactions_enabled = false);
    assert_eq!(reason(create(&closed, "t")).await, "COMMENTS_DISABLED");

    // Comments are not written on comments; they are replied to.
    let top = ctx.comment(&bob, &video, "top").await;
    assert_eq!(
        reason(CreateCommentRequest { subject: comment_ref(&top.id).into(), text: "t".into(), ..Default::default() })
            .await,
        "UNSUPPORTED_SUBJECT"
    );

    // Attachments and identities.
    let stamp = ctx.stamp(&bob).await;
    let image = ctx.image(&bob, AssetPurpose::ASSET_PURPOSE_COMMENT_IMAGE).await;
    let both =
        CreateCommentRequest { stamp_id: stamp.clone(), image_asset_id: image.to_string(), ..create(&video, "") };
    assert_eq!(reason(both).await, "ATTACHMENT_CONFLICT");
    let carols_stamp = ctx.stamp(&carol).await;
    assert_eq!(reason(CreateCommentRequest { stamp_id: carols_stamp, ..create(&video, "") }).await, "STAMP_NOT_USABLE");
    let carols_image = ctx.image(&carol, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await;
    assert_eq!(
        reason(CreateCommentRequest { image_asset_id: carols_image.to_string(), ..create(&video, "") }).await,
        "ASSET_NOT_FOUND"
    );
    let avatar = ctx.image(&bob, AssetPurpose::ASSET_PURPOSE_AVATAR).await;
    assert_eq!(
        reason(CreateCommentRequest { image_asset_id: avatar.to_string(), ..create(&video, "") }).await,
        "ASSET_WRONG_PURPOSE"
    );
    let clip = tokuly_media::testing::ready_video(&ctx.app.core, bob.id, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE)
        .await
        .unwrap();
    assert_eq!(
        reason(CreateCommentRequest { image_asset_id: clip.id.to_string(), ..create(&video, "") }).await,
        "ASSET_NOT_IMAGE"
    );
    assert_eq!(
        reason(CreateCommentRequest { image_asset_id: Uuid::now_v7().to_string(), ..create(&video, "") }).await,
        "ASSET_NOT_FOUND"
    );
    assert_eq!(
        reason(CreateCommentRequest { stamp_id: Uuid::now_v7().to_string(), ..create(&video, "") }).await,
        "STAMP_NOT_USABLE"
    );
    let carols_channel = ctx.channels.add(&carol, "Carol TV");
    assert_eq!(
        reason(CreateCommentRequest { channel_id: carols_channel.to_string(), ..create(&video, "t") }).await,
        "CHANNEL_NOT_FOUND"
    );
    // A picture uploaded for comments, or (as clients did before comments
    // had a purpose of their own) for a note or a chat.
    for purpose in [
        AssetPurpose::ASSET_PURPOSE_COMMENT_IMAGE,
        AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE,
        AssetPurpose::ASSET_PURPOSE_CHAT_IMAGE,
    ] {
        let picture = ctx.image(&bob, purpose).await;
        let text = format!("写真 {}", purpose as i32);
        let request = CreateCommentRequest { image_asset_id: picture.to_string(), ..create(&video, &text) };
        let posted: CreateCommentResponse = ctx.call(SVC, "CreateComment", Some(&bob), &request).await.unwrap();
        assert_eq!(posted.comment.image.asset_id, picture.to_string(), "{purpose:?}");
    }

    // The same comment twice in a row, however it is typed.
    ctx.comment(&bob, &video, "Hello").await;
    assert_eq!(reason(create(&video, "ｈｅｌｌｏ")).await, "DUPLICATE_COMMENT");
}

#[tokio::test]
async fn too_many_mentions_are_refused() {
    let ctx = start().await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    let mut handles = Vec::new();
    for index in 0..11 {
        handles.push(format!("@{}", ctx.app.user(&format!("friend{index}")).await.handle));
    }
    assert_eq!(
        ctx.reason(SVC, "CreateComment", Some(&bob), &create(&video, &handles.join(" "))).await,
        "TOO_MANY_MENTIONS"
    );
    let ten = ctx.comment(&bob, &video, &handles[..10].join(" ")).await;
    assert_eq!(ten.mentions.len(), 10);
    // An edit is held to the same limit.
    let edit = UpdateCommentRequest { comment_id: ten.id.clone(), text: handles.join(" "), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "UpdateComment", Some(&bob), &edit).await, "TOO_MANY_MENTIONS");
}

#[tokio::test]
async fn replies_form_one_level_threads() {
    let ctx = start().await;
    let (owner, bob, carol, dave) = (
        ctx.app.user("Owner").await,
        ctx.app.user("Bob").await,
        ctx.app.user("Carol").await,
        ctx.app.user("Dave").await,
    );
    let video = ctx.video(&owner, "v");
    let top = ctx.comment(&bob, &video, "スレッドの先頭").await;

    let first = ctx.reply(&carol, &top.id, "一つ目の返信").await;
    assert_eq!(first.parent_id, top.id);
    assert!(first.reply_to.as_option().is_none(), "a reply to the top-level comment answers nobody else");
    // Replying to a reply stays in the same thread and remembers whom it answers.
    let second = ctx.reply(&dave, &first.id, "二つ目の返信").await;
    assert_eq!(second.parent_id, top.id);
    assert_eq!(second.reply_to.user.id, carol.id.to_string());
    let third = ctx.reply(&bob, &top.id, "三つ目の返信").await;

    // Oldest first, in pages.
    let list = |cursor: String| {
        let ctx = &ctx;
        let top_id = top.id.clone();
        async move {
            let request = ListRepliesRequest { comment_id: top_id, page: page(2, &cursor), ..Default::default() };
            ctx.call::<_, ListRepliesResponse>(SVC, "ListReplies", None, &request).await.unwrap()
        }
    };
    let first_page = list(String::new()).await;
    assert_eq!(ids(&first_page.replies), [first.id.as_str(), second.id.as_str()]);
    assert!(first_page.page.has_more);
    let second_page = list(first_page.page.next_cursor.clone()).await;
    assert_eq!(ids(&second_page.replies), [third.id.as_str()]);
    assert!(!second_page.page.has_more);
    let bad_cursor = ListRepliesRequest { comment_id: top.id.clone(), page: page(2, "!!!"), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "ListReplies", None, &bad_cursor).await, "INVALID_CURSOR");
    // A reply has no replies of its own.
    let of_reply = ListRepliesRequest { comment_id: first.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "ListReplies", None, &of_reply).await, "COMMENT_NOT_FOUND");

    // The thread shows its reply count; the subject counts every comment.
    let listed = ctx.comments(None, &video).await;
    assert_eq!((listed.comments.len(), listed.comments[0].reply_count, listed.total_count), (1, 3, 4));
    assert_eq!(ctx.videos.counter(video.id, "comment_count"), 4);

    // Who hears about what: the thread's author about replies, the answered
    // reply's author too; the subject owner only about the top-level comment.
    assert_eq!(ctx.notifications(bob.id, "engagement.reply").await.len(), 2, "carol's and dave's replies");
    let to_carol = ctx.notifications(carol.id, "engagement.reply").await;
    assert_eq!(to_carol.len(), 1);
    assert_eq!(to_carol[0]["actor_id"], serde_json::json!(dave.id));
    assert_eq!(to_carol[0]["title"]["en"], "Dave replied to your comment");
    assert_eq!(ctx.notification_kinds(owner.id).await, ["engagement.comment"]);

    // GetComment on a reply returns its thread.
    let fetched = ctx.get_comment(None, &second.id).await.unwrap();
    assert_eq!((fetched.comment.id.as_str(), fetched.parent.id.as_str()), (second.id.as_str(), top.id.as_str()));
    assert_eq!((fetched.subject.title.as_str(), fetched.subject.available), ("v", true));
    assert_eq!(fetched.subject.path, video.path);
}

#[tokio::test]
async fn mentions_are_linked_and_announced_to_those_who_can_see_the_subject() {
    let ctx = start().await;
    let (owner, bob, carol, dave) = (
        ctx.app.user("Owner").await,
        ctx.app.user("Bob").await,
        ctx.app.user("Carol").await,
        ctx.app.user("Dave").await,
    );
    let video = ctx.video(&owner, "friends only");
    ctx.videos.update(video.id, |subject| subject.audience = Audience::Only(vec![bob.id, carol.id]));

    let text = format!("@{} @{} @{} @nobody-here みて!", carol.handle, dave.handle, bob.handle);
    let comment = ctx.comment(&bob, &video, &text).await;
    // Mentions resolve to accounts; unknown handles and oneself are not mentions.
    let mut mentioned: Vec<&str> = comment.mentions.iter().map(|user| user.handle.as_str()).collect();
    mentioned.sort();
    let mut expected = [carol.handle.as_str(), dave.handle.as_str()];
    expected.sort();
    assert_eq!(mentioned, expected);

    let to_carol = ctx.notifications(carol.id, "engagement.mention").await;
    assert_eq!(to_carol.len(), 1);
    assert_eq!(to_carol[0]["title"]["ja"], "Bobがコメントであなたをメンションしたよ!");
    assert!(ctx.notification_kinds(dave.id).await.is_empty(), "dave cannot see the subject");
    assert!(ctx.notification_kinds(bob.id).await.is_empty());
}

#[tokio::test]
async fn comments_carry_a_stamp_or_an_image() {
    let ctx = start().await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    let core = &ctx.app.core;

    let stamp_id = ctx.stamp(&bob).await;
    let request = CreateCommentRequest { stamp_id: stamp_id.clone(), ..create(&video, "") };
    let with_stamp: CreateCommentResponse = ctx.call(SVC, "CreateComment", Some(&bob), &request).await.unwrap();
    assert_eq!(with_stamp.comment.stamp.id, stamp_id);
    assert!(!with_stamp.comment.stamp.image_url.is_empty() && with_stamp.comment.text.is_empty());
    // The send reached the stamp domain's usage log.
    let sends: i64 = sqlx::query_scalar("SELECT count(*) FROM stamp_usage WHERE user_id = $1 AND context = 3")
        .bind(bob.id)
        .fetch_one(&core.db)
        .await
        .unwrap();
    assert_eq!(sends, 1);
    assert_eq!(ctx.notifications(owner.id, "engagement.comment").await[0]["body"]["ja"], "スタンプを送ったよ");

    let asset_id = ctx.image(&bob, AssetPurpose::ASSET_PURPOSE_CHAT_IMAGE).await;
    let request = CreateCommentRequest { image_asset_id: asset_id.to_string(), ..create(&video, "写真だよ") };
    let with_image: CreateCommentResponse = ctx.call(SVC, "CreateComment", Some(&bob), &request).await.unwrap();
    assert_eq!(with_image.comment.image.asset_id, asset_id.to_string());
    assert!(!with_image.comment.image.url.is_empty());
    // The comment holds its upload for as long as it exists.
    assert!(tokuly_media::assets::is_attached(core, asset_id).await.unwrap());
    let delete = DeleteCommentRequest { comment_id: with_image.comment.id.clone(), ..Default::default() };
    let _: DeleteCommentResponse = ctx.call(SVC, "DeleteComment", Some(&bob), &delete).await.unwrap();
    assert!(!tokuly_media::assets::is_attached(core, asset_id).await.unwrap());

    // A stamp that has since been deleted leaves its ID behind.
    let gone = tokuly_proto::stamp::DeletePersonalStampRequest { stamp_id: stamp_id.clone(), ..Default::default() };
    let _: tokuly_proto::stamp::DeletePersonalStampResponse =
        ctx.app.call("tokuly.stamp.v1.StampLibraryService/DeletePersonalStamp", Some(&bob), &gone).await.unwrap();
    let fetched = ctx.get_comment(None, &with_stamp.comment.id).await.unwrap();
    assert_eq!(fetched.comment.stamp.id, stamp_id);
    assert!(fetched.comment.stamp.image_url.is_empty());
}

#[tokio::test]
async fn a_comment_written_as_a_channel_hides_the_account() {
    let ctx = start().await;
    let (owner, bob, carol) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await, ctx.app.user("Carol").await);
    let video = ctx.video(&owner, "v");
    let channel = ctx.channels.add(&bob, "Bob Games");

    let request =
        CreateCommentRequest { channel_id: channel.to_string(), ..create(&video, "チャンネルから失礼!") };
    let created: CreateCommentResponse = ctx.call(SVC, "CreateComment", Some(&bob), &request).await.unwrap();
    // The author sees both of their identities.
    assert_eq!(created.comment.author.channel.name, "Bob Games");
    assert_eq!(created.comment.author.user.id, bob.id.to_string());

    // Nobody else learns which account it is: not readers, not the owner.
    for viewer in [None, Some(&carol), Some(&owner)] {
        let fetched = ctx.get_comment(viewer, &created.comment.id).await.unwrap();
        assert_eq!(fetched.comment.author.channel.id, channel.to_string());
        assert!(fetched.comment.author.user.as_option().is_none());
    }
    // Neither does the notification: no actor, the channel's name and icon.
    let notice = &ctx.notifications(owner.id, "engagement.comment").await[0];
    assert!(notice["actor_id"].is_null());
    assert_eq!(notice["title"]["en"], "Bob Games commented on \"v\"");
    assert_eq!(notice["image_url"], format!("https://media.test/channel/{channel}.webp"));

    // A reply to it reaches the account behind the channel all the same.
    ctx.reply(&carol, &created.comment.id, "こんにちは").await;
    assert_eq!(ctx.notifications(bob.id, "engagement.reply").await.len(), 1);
}

#[tokio::test]
async fn top_level_comments_sort_by_score_or_time_and_paginate() {
    let ctx = start().await;
    let (owner, bob, carol) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await, ctx.app.user("Carol").await);
    let video = ctx.video(&owner, "v");
    let oldest = ctx.comment(&bob, &video, "いちばん古い").await;
    let middle = ctx.comment(&bob, &video, "まんなか").await;
    let newest = ctx.comment(&bob, &video, "いちばん新しい").await;
    // A like lifts the oldest comment above the newer ones in the "top" order.
    ctx.react(&carol, comment_ref(&oldest.id), like()).await;

    let list = |sort: CommentSort, limit: i32, cursor: String| {
        let ctx = &ctx;
        let subject = video.subject_ref();
        async move {
            let request = ListCommentsRequest {
                subject: subject.into(),
                sort: sort.into(),
                page: page(limit, &cursor),
                ..Default::default()
            };
            ctx.call::<_, ListCommentsResponse>(SVC, "ListComments", None, &request).await.unwrap()
        }
    };
    let newest_first = list(CommentSort::COMMENT_SORT_NEWEST, 0, String::new()).await;
    assert_eq!(ids(&newest_first.comments), [newest.id.as_str(), middle.id.as_str(), oldest.id.as_str()]);
    for sort in [CommentSort::COMMENT_SORT_TOP, CommentSort::COMMENT_SORT_UNSPECIFIED] {
        let top = list(sort, 0, String::new()).await;
        assert_eq!(ids(&top.comments), [oldest.id.as_str(), newest.id.as_str(), middle.id.as_str()]);
        assert_eq!(top.comments[0].like_count, 1);
    }

    // Both orders page through the whole list without repeating anything.
    for sort in [CommentSort::COMMENT_SORT_TOP, CommentSort::COMMENT_SORT_NEWEST] {
        let mut seen: Vec<String> = Vec::new();
        let mut cursor = String::new();
        loop {
            let page = list(sort, 2, cursor).await;
            seen.extend(page.comments.iter().map(|comment| comment.id.clone()));
            if !page.page.has_more {
                break;
            }
            cursor = page.page.next_cursor.clone();
        }
        assert_eq!(seen.len(), 3);
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 3);
    }
    // A cursor of one order is not a cursor of the other.
    let top_cursor = list(CommentSort::COMMENT_SORT_TOP, 1, String::new()).await.page.next_cursor.clone();
    let mixed = ListCommentsRequest {
        subject: video.subject_ref().into(),
        sort: CommentSort::COMMENT_SORT_NEWEST.into(),
        page: page(1, &top_cursor),
        ..Default::default()
    };
    assert_eq!(ctx.reason(SVC, "ListComments", None, &mixed).await, "INVALID_CURSOR");
}

#[tokio::test]
async fn authors_edit_their_own_comments() {
    let ctx = start().await;
    let (owner, bob, carol) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await, ctx.app.user("Carol").await);
    let video = ctx.video(&owner, "v");
    let comment = ctx.comment(&bob, &video, "さいしょの文").await;
    let mut topic = ctx.app.listen_topic(&video.topic()).await;

    let edit = |text: String| UpdateCommentRequest { comment_id: comment.id.clone(), text, ..Default::default() };
    assert_eq!(ctx.reason(SVC, "UpdateComment", None, &edit("x".into())).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.reason(SVC, "UpdateComment", Some(&carol), &edit("x".into())).await, "NOT_COMMENT_AUTHOR");
    assert_eq!(ctx.reason(SVC, "UpdateComment", Some(&owner), &edit("x".into())).await, "NOT_COMMENT_AUTHOR");
    assert_eq!(ctx.reason(SVC, "UpdateComment", Some(&bob), &edit("  ".into())).await, "COMMENT_EMPTY");
    assert_eq!(ctx.reason(SVC, "UpdateComment", Some(&bob), &edit("x".repeat(1001))).await, "COMMENT_TOO_LONG");
    let missing =
        UpdateCommentRequest { comment_id: Uuid::now_v7().to_string(), text: "x".into(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "UpdateComment", Some(&bob), &missing).await, "COMMENT_NOT_FOUND");

    let edited: UpdateCommentResponse =
        ctx.call(SVC, "UpdateComment", Some(&bob), &edit(format!("なおした文 @{}", carol.handle))).await.unwrap();
    assert!(edited.comment.edited && edited.comment.edited_at.as_option().is_some());
    assert_eq!(edited.comment.mentions.len(), 1);
    assert_eq!(edited.comment.status, CommentStatus::COMMENT_STATUS_PUBLISHED);
    // The person newly mentioned hears about it; watchers get the new text.
    assert_eq!(ctx.notifications(carol.id, "engagement.mention").await.len(), 1);
    let events = drain(&mut topic).await;
    assert!(events.iter().any(
        |event| matches!(event, Pushed::CommentUpdated(updated) if updated.comment.text.starts_with("なおした文"))
    ));

    // Saving the same text again changes nothing; mentioning the same person again is not news.
    let again: UpdateCommentResponse =
        ctx.call(SVC, "UpdateComment", Some(&bob), &edit(format!("なおした文 @{} !", carol.handle))).await.unwrap();
    assert!(again.comment.text.ends_with('!'));
    assert_eq!(ctx.notifications(carol.id, "engagement.mention").await.len(), 1);
}

#[tokio::test]
async fn authors_and_owners_delete_comments_with_their_replies() {
    let ctx = start().await;
    let (owner, bob, carol) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await, ctx.app.user("Carol").await);
    let video = ctx.video(&owner, "v");
    let top = ctx.comment(&bob, &video, "消されるスレッド").await;
    let reply = ctx.reply(&carol, &top.id, "返信1").await;
    ctx.reply(&bob, &top.id, "返信2").await;
    let other = ctx.comment(&carol, &video, "残るコメント").await;
    ctx.react(&carol, comment_ref(&top.id), like()).await;
    assert_eq!(ctx.videos.counter(video.id, "comment_count"), 4);
    let mut topic = ctx.app.listen_topic(&video.topic()).await;

    let delete = |id: &str| DeleteCommentRequest { comment_id: id.to_owned(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "DeleteComment", None, &delete(&top.id)).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.reason(SVC, "DeleteComment", Some(&carol), &delete(&top.id)).await, "COMMENT_DELETE_FORBIDDEN");
    assert_eq!(
        ctx.reason(SVC, "DeleteComment", Some(&bob), &delete(&Uuid::now_v7().to_string())).await,
        "COMMENT_NOT_FOUND"
    );

    // A reply goes alone.
    let one: DeleteCommentResponse = ctx.call(SVC, "DeleteComment", Some(&carol), &delete(&reply.id)).await.unwrap();
    assert_eq!(one.removed_count, 1);
    assert_eq!(ctx.comments(None, &video).await.comments.iter().find(|c| c.id == top.id).unwrap().reply_count, 1);
    // The owner deletes someone else's thread: it goes with what is left below it.
    let thread: DeleteCommentResponse = ctx.call(SVC, "DeleteComment", Some(&owner), &delete(&top.id)).await.unwrap();
    assert_eq!(thread.removed_count, 2);

    let listed = ctx.comments(None, &video).await;
    assert_eq!((ids(&listed.comments), listed.total_count), (vec![other.id.as_str()], 1));
    assert_eq!(ctx.videos.counter(video.id, "comment_count"), 1);
    assert_eq!(ctx.get_comment(Some(&bob), &top.id).await.unwrap_err().reason, "COMMENT_NOT_FOUND");
    // What hung off the deleted comments is gone as well.
    let likes: i64 = sqlx::query_scalar("SELECT count(*) FROM reactions WHERE subject_id = $1")
        .bind(Uuid::parse_str(&top.id).unwrap())
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(likes, 0);

    let removed: Vec<i32> = drain(&mut topic)
        .await
        .iter()
        .filter_map(|event| match event {
            Pushed::CommentRemoved(removed) => Some(removed.removed_count),
            _ => None,
        })
        .collect();
    assert_eq!(removed, [1, 2]);
    let deleted = ctx.events_about("engagement.comment.deleted", video.id).await;
    assert_eq!(deleted.len(), 2);
}

#[tokio::test]
async fn owners_pin_one_comment_and_give_hearts() {
    let ctx = start().await;
    let (owner, bob, carol) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await, ctx.app.user("Carol").await);
    let video = ctx.video(&owner, "v");
    let first = ctx.comment(&bob, &video, "ひとつめ").await;
    let second = ctx.comment(&carol, &video, "ふたつめ").await;
    let third = ctx.comment(&bob, &video, "みっつめ").await;
    let reply = ctx.reply(&carol, &first.id, "返信").await;
    let mut topic = ctx.app.listen_topic(&video.topic()).await;

    let pin = |id: &str| PinCommentRequest { comment_id: id.to_owned(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "PinComment", None, &pin(&first.id)).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.reason(SVC, "PinComment", Some(&bob), &pin(&first.id)).await, "NOT_SUBJECT_OWNER");
    assert_eq!(ctx.reason(SVC, "PinComment", Some(&owner), &pin(&reply.id)).await, "COMMENT_NOT_PINNABLE");
    assert_eq!(
        ctx.reason(SVC, "PinComment", Some(&owner), &pin(&Uuid::now_v7().to_string())).await,
        "COMMENT_NOT_FOUND"
    );

    let pinned: PinCommentResponse = ctx.call(SVC, "PinComment", Some(&owner), &pin(&first.id)).await.unwrap();
    assert!(pinned.comment.pinned);
    // The pinned comment leads the first page, whatever the order.
    assert_eq!(
        ids(&ctx.comments(None, &video).await.comments),
        [first.id.as_str(), third.id.as_str(), second.id.as_str()]
    );
    // Pinning another one replaces it.
    let _: PinCommentResponse = ctx.call(SVC, "PinComment", Some(&owner), &pin(&second.id)).await.unwrap();
    let listed = ctx.comments(None, &video).await;
    assert_eq!(ids(&listed.comments), [second.id.as_str(), third.id.as_str(), first.id.as_str()]);
    assert_eq!(listed.comments.iter().filter(|comment| comment.pinned).count(), 1);
    let pins: Vec<(String, bool)> = drain(&mut topic)
        .await
        .iter()
        .filter_map(|event| match event {
            Pushed::CommentPinned(pinned) => Some((pinned.comment_id.clone(), pinned.pinned)),
            _ => None,
        })
        .collect();
    assert_eq!(pins, [(first.id.clone(), true), (first.id.clone(), false), (second.id.clone(), true)]);

    let unpin = UnpinCommentRequest { comment_id: second.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "UnpinComment", None, &unpin).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.reason(SVC, "UnpinComment", Some(&carol), &unpin).await, "NOT_SUBJECT_OWNER");
    let unknown = UnpinCommentRequest { comment_id: Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "UnpinComment", Some(&owner), &unknown).await, "COMMENT_NOT_FOUND");
    let unpinned: UnpinCommentResponse = ctx.call(SVC, "UnpinComment", Some(&owner), &unpin).await.unwrap();
    assert!(!unpinned.comment.pinned);
    assert_eq!(
        ids(&ctx.comments(None, &video).await.comments),
        [third.id.as_str(), second.id.as_str(), first.id.as_str()]
    );

    // Hearts.
    let heart =
        |id: &str, hearted: bool| SetCommentHeartRequest { comment_id: id.to_owned(), hearted, ..Default::default() };
    assert_eq!(ctx.reason(SVC, "SetCommentHeart", None, &heart(&first.id, true)).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.reason(SVC, "SetCommentHeart", Some(&bob), &heart(&first.id, true)).await, "NOT_SUBJECT_OWNER");
    assert_eq!(
        ctx.reason(SVC, "SetCommentHeart", Some(&owner), &heart(&Uuid::now_v7().to_string(), true)).await,
        "COMMENT_NOT_FOUND"
    );
    let hearted: SetCommentHeartResponse =
        ctx.call(SVC, "SetCommentHeart", Some(&owner), &heart(&first.id, true)).await.unwrap();
    assert!(hearted.comment.hearted);
    let notices = ctx.notifications(bob.id, "engagement.heart").await;
    assert_eq!(notices.len(), 1);
    assert!(notices[0]["actor_id"].is_null(), "the creator is not named as a person");
    assert_eq!(notices[0]["body"]["ja"], "ひとつめ");
    assert!(
        drain(&mut topic)
            .await
            .iter()
            .any(|event| matches!(event, Pushed::CommentUpdated(updated) if updated.comment.hearted))
    );
    // A heart outweighs recency in the "top" order.
    let top = ListCommentsRequest { subject: video.subject_ref().into(), ..Default::default() };
    let top: ListCommentsResponse = ctx.call(SVC, "ListComments", None, &top).await.unwrap();
    assert_eq!(top.comments[0].id, first.id);
    // Taking it back and giving it again is not news twice.
    let _: SetCommentHeartResponse =
        ctx.call(SVC, "SetCommentHeart", Some(&owner), &heart(&first.id, false)).await.unwrap();
    let again: SetCommentHeartResponse =
        ctx.call(SVC, "SetCommentHeart", Some(&owner), &heart(&first.id, true)).await.unwrap();
    assert!(again.comment.hearted);
    assert_eq!(ctx.notifications(bob.id, "engagement.heart").await.len(), 1);
}

#[tokio::test]
async fn my_comments_come_with_what_they_are_on() {
    let ctx = start().await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    let (video, note) = (ctx.video(&owner, "動画"), ctx.note(&owner, "ノート"));
    let on_video = ctx.comment(&bob, &video, "動画へ").await;
    let on_note = ctx.comment(&bob, &note, "ノートへ").await;
    ctx.comment(&owner, &video, "ほかの人の").await;

    assert_eq!(ctx.reason(SVC, "ListMyComments", None, &ListMyCommentsRequest::default()).await, "SIGN_IN_REQUIRED");
    let mine: ListMyCommentsResponse =
        ctx.call(SVC, "ListMyComments", Some(&bob), &ListMyCommentsRequest::default()).await.unwrap();
    let listed: Vec<(&str, &str, bool)> = mine
        .comments
        .iter()
        .map(|entry| (entry.comment.id.as_str(), entry.subject.title.as_str(), entry.subject.available))
        .collect();
    assert_eq!(listed, [(on_note.id.as_str(), "ノート", true), (on_video.id.as_str(), "動画", true)]);

    // A subject that is gone leaves the comment listed, marked unavailable —
    // and its author can still delete it.
    ctx.notes.remove(note.id);
    let paged = ListMyCommentsRequest { page: page(1, ""), ..Default::default() };
    let mine: ListMyCommentsResponse = ctx.call(SVC, "ListMyComments", Some(&bob), &paged).await.unwrap();
    assert_eq!(mine.comments.len(), 1);
    assert!(mine.page.has_more);
    assert!(!mine.comments[0].subject.available && mine.comments[0].subject.title.is_empty());
    assert_eq!(mine.comments[0].subject.subject.id, note.id.to_string());
    assert_eq!(ctx.get_comment(Some(&bob), &on_note.id).await.unwrap_err().reason, "COMMENT_NOT_FOUND");
    let delete = DeleteCommentRequest { comment_id: on_note.id.clone(), ..Default::default() };
    let removed: DeleteCommentResponse = ctx.call(SVC, "DeleteComment", Some(&bob), &delete).await.unwrap();
    assert_eq!(removed.removed_count, 1);
}

#[tokio::test]
async fn blocked_users_neither_reach_nor_see_each_other() {
    let ctx = start().await;
    let (owner, alice, bob, carol) = (
        ctx.app.user("Owner").await,
        ctx.app.user("Alice").await,
        ctx.app.user("Bob").await,
        ctx.app.user("Carol").await,
    );
    let video = ctx.video(&owner, "v");
    let from_alice = ctx.comment(&alice, &video, "アリスのコメント").await;
    let from_bob = ctx.comment(&bob, &video, "ボブのコメント").await;
    let bobs_reply = ctx.reply(&bob, &from_alice.id, "ボブの返信").await;
    let alices_video = ctx.video(&alice, "alice's");

    ctx.block(&alice, &bob).await;

    // Neither sees the other's comments; everyone else sees both.
    assert_eq!(texts(&ctx.comments(Some(&alice), &video).await.comments), ["アリスのコメント"]);
    assert_eq!(texts(&ctx.comments(Some(&bob), &video).await.comments), ["ボブのコメント"]);
    assert_eq!(ctx.comments(Some(&carol), &video).await.comments.len(), 2);
    assert_eq!(ctx.get_comment(Some(&alice), &from_bob.id).await.unwrap_err().reason, "COMMENT_NOT_FOUND");
    assert_eq!(ctx.get_comment(Some(&bob), &from_alice.id).await.unwrap_err().reason, "COMMENT_NOT_FOUND");
    let replies = ListRepliesRequest { comment_id: from_alice.id.clone(), ..Default::default() };
    let seen_by_alice: ListRepliesResponse = ctx.call(SVC, "ListReplies", Some(&alice), &replies).await.unwrap();
    assert!(seen_by_alice.replies.is_empty());
    let seen_by_carol: ListRepliesResponse = ctx.call(SVC, "ListReplies", Some(&carol), &replies).await.unwrap();
    assert_eq!(ids(&seen_by_carol.replies), [bobs_reply.id.as_str()]);

    // Bob cannot reach alice's subjects or comments at all.
    assert_eq!(ctx.reason(SVC, "CreateComment", Some(&bob), &create(&alices_video, "やあ")).await, "SUBJECT_NOT_FOUND");
    let reply = CreateCommentRequest { parent_id: from_alice.id.clone(), text: "やあ".into(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "CreateComment", Some(&bob), &reply).await, "COMMENT_NOT_FOUND");
    let list = ListCommentsRequest { subject: alices_video.subject_ref().into(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "ListComments", Some(&bob), &list).await, "SUBJECT_NOT_FOUND");

    // Mentions across a block are dropped silently.
    let mention = ctx.comment(&bob, &video, &format!("@{} きこえる?", alice.handle)).await;
    assert!(mention.mentions.is_empty());
    assert!(ctx.notifications(alice.id, "engagement.mention").await.is_empty());

    // On a topic everyone listens to, a comment of someone with blocks in
    // place travels as an ID only: subscribers fetch it, and the blocked
    // ones are refused.
    let mut topic = ctx.app.listen_topic(&video.topic()).await;
    let quiet = ctx.comment(&bob, &video, "これは誰に届く?").await;
    let events = drain(&mut topic).await;
    let added = events.iter().find_map(|event| match event {
        Pushed::CommentAdded(added) => Some(added),
        _ => None,
    });
    let added = added.expect("a CommentAdded push");
    assert_eq!(added.comment_id, quiet.id);
    assert!(added.comment.as_option().is_none());
}

#[tokio::test]
async fn turned_off_comments_stay_out_of_sight() {
    let ctx = start().await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    let comment = ctx.comment(&bob, &video, "まだ開いてた頃").await;
    ctx.videos.update(video.id, |subject| subject.interactions_enabled = false);

    for viewer in [None, Some(&bob)] {
        let listed = ctx.comments(viewer, &video).await;
        assert!(listed.comments.is_empty() && !listed.comments_enabled && listed.total_count == 0);
    }
    let as_owner = ctx.comments(Some(&owner), &video).await;
    assert_eq!(ids(&as_owner.comments), [comment.id.as_str()]);
    assert!(!as_owner.comments_enabled);
    let reply = CreateCommentRequest { parent_id: comment.id.clone(), text: "返信".into(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "CreateComment", Some(&bob), &reply).await, "COMMENTS_DISABLED");
}

#[tokio::test]
async fn commenting_is_rate_limited() {
    let ctx = start_with(EngagementConfig { comments_per_minute: 2, ..test_config() }).await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    ctx.comment(&bob, &video, "one").await;
    ctx.comment(&bob, &video, "two").await;
    assert_eq!(ctx.reason(SVC, "CreateComment", Some(&bob), &create(&video, "three")).await, "RATE_LIMITED");
    // The limit is per user.
    ctx.comment(&owner, &video, "mine").await;
}
