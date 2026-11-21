//! `CommentModerationService`: the owner's review queue, screening rules
//! and blocked commenters.
mod common;

use common::*;
use tokuly_proto::{
    engagement::{
        __buffa::oneof::{block_commenter_request::Target, engagement_event::Event as Pushed},
        *,
    },
    realtime::server_frame::Body,
};
use uuid::Uuid;

const SVC: &str = "CommentModerationService";

fn received(status: CommentStatus) -> ListReceivedCommentsRequest {
    ListReceivedCommentsRequest { status: status.into(), ..Default::default() }
}

fn by_id(comment_id: &str) -> ApproveCommentRequest {
    ApproveCommentRequest { comment_id: comment_id.to_owned(), ..Default::default() }
}

fn hide(comment_id: &str) -> HideCommentRequest {
    HideCommentRequest { comment_id: comment_id.to_owned(), ..Default::default() }
}

fn block_user(user_id: Uuid) -> BlockCommenterRequest {
    BlockCommenterRequest { target: Some(Target::UserId(user_id.to_string())), ..Default::default() }
}

fn block_author_of(comment_id: &str) -> BlockCommenterRequest {
    BlockCommenterRequest { target: Some(Target::CommentId(comment_id.to_owned())), ..Default::default() }
}

impl Ctx {
    async fn received(&self, owner: &tokuly_testkit::TestUser, request: &ListReceivedCommentsRequest) -> ListReceivedCommentsResponse {
        self.call(SVC, "ListReceivedComments", Some(owner), request).await.unwrap()
    }
}

fn received_texts(response: &ListReceivedCommentsResponse) -> Vec<&str> {
    response.comments.iter().map(|entry| entry.comment.text.as_str()).collect()
}

#[tokio::test]
async fn held_comments_wait_in_the_owners_review_queue() {
    let ctx = start().await;
    let (owner, bob, carol) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await, ctx.app.user("Carol").await);
    let video = ctx.video(&owner, "v");
    ctx.set_rules(&owner, true, false, &[]).await;
    let mut owner_frames = ctx.app.listen_user(owner.id).await;
    let mut topic = ctx.app.listen_topic(&video.topic()).await;

    // The comment is stored but waits; only its author and the owner see it.
    let held = ctx.comment(&bob, &video, &format!("承認してね @{}", carol.handle)).await;
    assert_eq!(held.status, CommentStatus::COMMENT_STATUS_HELD);
    assert_eq!(held.hold_reason, HoldReason::HOLD_REASON_UNSPECIFIED, "the author is not told why");
    let for_guests = ctx.comments(None, &video).await;
    assert!(for_guests.comments.is_empty() && for_guests.total_count == 0);
    assert_eq!(ctx.comments(Some(&bob), &video).await.comments[0].status, CommentStatus::COMMENT_STATUS_HELD);
    assert_eq!(ctx.get_comment(Some(&carol), &held.id).await.unwrap_err().reason, "COMMENT_NOT_FOUND");
    assert_eq!(ctx.get_comment(Some(&owner), &held.id).await.unwrap().comment.hold_reason, HoldReason::HOLD_REASON_REVIEW_ALL);
    assert_eq!(ctx.videos.counter(video.id, "comment_count"), 0);
    // Nothing went out on the public topic, and nobody mentioned was told yet.
    assert!(drain(&mut topic).await.is_empty());
    assert!(ctx.notification_kinds(carol.id).await.is_empty());
    // The owner's badge moved, and they got one "waiting for review" entry.
    let badge = owner_frames.next(std::time::Duration::from_secs(2)).await.expect("a review queue push");
    match badge.body {
        Some(Body::Engagement(event)) => match &event.event {
            Some(Pushed::ReviewQueueChanged(queue)) => assert_eq!(queue.held_count, 1),
            other => panic!("unexpected event {other:?}"),
        },
        other => panic!("unexpected frame {other:?}"),
    }
    let notices = ctx.notifications(owner.id, "engagement.comment").await;
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0]["title"]["en"], "A comment is waiting for your review");
    // The owner's own comments are never screened.
    assert_eq!(ctx.comment(&owner, &video, "自分のコメント").await.status, CommentStatus::COMMENT_STATUS_PUBLISHED);

    // The queue.
    assert_eq!(ctx.reason(SVC, "ListReceivedComments", None, &received(CommentStatus::COMMENT_STATUS_HELD)).await, "SIGN_IN_REQUIRED");
    let queue = ctx.received(&owner, &received(CommentStatus::COMMENT_STATUS_HELD)).await;
    assert_eq!(queue.held_count, 1);
    assert_eq!(queue.comments.len(), 1);
    let entry = &queue.comments[0];
    assert_eq!((entry.comment.id.as_str(), entry.subject.title.as_str()), (held.id.as_str(), "v"));
    assert_eq!(entry.comment.hold_reason, HoldReason::HOLD_REASON_REVIEW_ALL);
    assert!(entry.comment.viewer.can_moderate);
    assert!(ctx.received(&bob, &received(CommentStatus::COMMENT_STATUS_HELD)).await.comments.is_empty(), "bob owns nothing");

    // Approval publishes it: counted, pushed, and announced only now.
    assert_eq!(ctx.reason(SVC, "ApproveComment", None, &by_id(&held.id)).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.reason(SVC, "ApproveComment", Some(&bob), &by_id(&held.id)).await, "NOT_SUBJECT_OWNER");
    assert_eq!(ctx.reason(SVC, "ApproveComment", Some(&carol), &by_id(&held.id)).await, "COMMENT_NOT_FOUND");
    let approved: ApproveCommentResponse = ctx.call(SVC, "ApproveComment", Some(&owner), &by_id(&held.id)).await.unwrap();
    assert_eq!(approved.comment.status, CommentStatus::COMMENT_STATUS_PUBLISHED);
    assert_eq!(ctx.comments(None, &video).await.total_count, 2);
    assert_eq!(ctx.videos.counter(video.id, "comment_count"), 2);
    assert!(drain(&mut topic).await.iter().any(|event| matches!(event, Pushed::CommentAdded(added) if added.comment_id == held.id)));
    assert_eq!(ctx.notifications(carol.id, "engagement.mention").await.len(), 1);
    assert_eq!(ctx.received(&owner, &received(CommentStatus::COMMENT_STATUS_HELD)).await.held_count, 0);
    assert_eq!(ctx.events_about("engagement.comment.published", video.id).await.len(), 2);

    // Approving again changes nothing and tells nobody twice.
    let _: ApproveCommentResponse = ctx.call(SVC, "ApproveComment", Some(&owner), &by_id(&held.id)).await.unwrap();
    assert_eq!(ctx.videos.counter(video.id, "comment_count"), 2);
    assert_eq!(ctx.notifications(carol.id, "engagement.mention").await.len(), 1);
}

#[tokio::test]
async fn screening_rules_hold_words_and_links() {
    let ctx = start().await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");

    // Rules start empty and are stored trimmed and without duplicates.
    assert_eq!(ctx.reason(SVC, "GetModerationSettings", None, &GetModerationSettingsRequest::default()).await, "SIGN_IN_REQUIRED");
    let initial: GetModerationSettingsResponse =
        ctx.call(SVC, "GetModerationSettings", Some(&owner), &GetModerationSettingsRequest::default()).await.unwrap();
    assert!(!initial.settings.hold_all && !initial.settings.hold_links && initial.settings.blocked_words.is_empty());
    let update = |words: Vec<String>, hold_links: bool| UpdateModerationSettingsRequest {
        settings: ModerationSettings { hold_links, blocked_words: words, ..Default::default() }.into(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(SVC, "UpdateModerationSettings", None, &update(vec![], false)).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.reason(SVC, "UpdateModerationSettings", Some(&owner), &update(vec!["  ".into()], false)).await, "BLOCKED_WORDS_INVALID");
    assert_eq!(ctx.reason(SVC, "UpdateModerationSettings", Some(&owner), &update(vec!["x".repeat(51)], false)).await, "BLOCKED_WORDS_INVALID");
    let too_many: Vec<String> = (0..201).map(|index| format!("w{index}")).collect();
    assert_eq!(ctx.reason(SVC, "UpdateModerationSettings", Some(&owner), &update(too_many, false)).await, "BLOCKED_WORDS_INVALID");
    let stored: UpdateModerationSettingsResponse = ctx
        .call(SVC, "UpdateModerationSettings", Some(&owner), &update(vec![" スパム ".into(), "ｽﾊﾟﾑ".into(), "Buy Now".into()], true))
        .await
        .unwrap();
    assert_eq!(stored.settings.blocked_words, ["スパム", "Buy Now"]);
    let fetched: GetModerationSettingsResponse =
        ctx.call(SVC, "GetModerationSettings", Some(&owner), &GetModerationSettingsRequest::default()).await.unwrap();
    assert!(fetched.settings.hold_links && fetched.settings.blocked_words.len() == 2);

    // Words match whatever the case or width; links are held when asked for.
    let clean = ctx.comment(&bob, &video, "ふつうのコメント").await;
    assert_eq!(clean.status, CommentStatus::COMMENT_STATUS_PUBLISHED);
    let wordy = ctx.comment(&bob, &video, "ｂｕｙ　ｎｏｗ!!").await;
    let linky = ctx.comment(&bob, &video, "見てね example.com").await;
    assert!(wordy.status == CommentStatus::COMMENT_STATUS_HELD && linky.status == CommentStatus::COMMENT_STATUS_HELD);
    let queue = ctx.received(&owner, &received(CommentStatus::COMMENT_STATUS_HELD)).await;
    let reasons: Vec<(&str, HoldReason)> =
        queue.comments.iter().map(|entry| (entry.comment.id.as_str(), entry.comment.hold_reason.as_known().unwrap())).collect();
    assert_eq!(
        reasons,
        [(linky.id.as_str(), HoldReason::HOLD_REASON_LINK), (wordy.id.as_str(), HoldReason::HOLD_REASON_BLOCKED_WORD)]
    );
    assert_eq!(queue.held_count, 2);

    // An edit runs through the rules again: a published comment can come
    // back held, and leaves public view when it does.
    assert_eq!(ctx.videos.counter(video.id, "comment_count"), 1);
    let mut topic = ctx.app.listen_topic(&video.topic()).await;
    let edit = UpdateCommentRequest { comment_id: clean.id.clone(), text: "やっぱりスパム".into(), ..Default::default() };
    let edited: UpdateCommentResponse = ctx.call("CommentService", "UpdateComment", Some(&bob), &edit).await.unwrap();
    assert_eq!(edited.comment.status, CommentStatus::COMMENT_STATUS_HELD);
    assert_eq!(ctx.videos.counter(video.id, "comment_count"), 0);
    assert!(ctx.comments(None, &video).await.comments.is_empty());
    assert!(drain(&mut topic).await.iter().any(|event| matches!(event, Pushed::CommentRemoved(removed) if removed.comment_id == clean.id)));
    // But an edit never publishes: cleaning up a held comment leaves it held.
    let cleaned = UpdateCommentRequest { comment_id: wordy.id.clone(), text: "きれいにした".into(), ..Default::default() };
    let still: UpdateCommentResponse = ctx.call("CommentService", "UpdateComment", Some(&bob), &cleaned).await.unwrap();
    assert_eq!(still.comment.status, CommentStatus::COMMENT_STATUS_HELD);
}

#[tokio::test]
async fn owners_hide_and_bring_back_comments() {
    let ctx = start().await;
    let (owner, bob, carol) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await, ctx.app.user("Carol").await);
    let video = ctx.video(&owner, "v");
    let top = ctx.comment(&bob, &video, "かくされるスレッド").await;
    let reply = ctx.reply(&carol, &top.id, "返信").await;
    ctx.comment(&carol, &video, "のこるコメント").await;
    let pin = PinCommentRequest { comment_id: top.id.clone(), ..Default::default() };
    let _: PinCommentResponse = ctx.call("CommentService", "PinComment", Some(&owner), &pin).await.unwrap();
    assert_eq!(ctx.videos.counter(video.id, "comment_count"), 3);
    let mut topic = ctx.app.listen_topic(&video.topic()).await;

    assert_eq!(ctx.reason(SVC, "HideComment", None, &hide(&top.id)).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.reason(SVC, "HideComment", Some(&bob), &hide(&top.id)).await, "NOT_SUBJECT_OWNER");
    assert_eq!(ctx.reason(SVC, "HideComment", Some(&owner), &hide(&Uuid::now_v7().to_string())).await, "COMMENT_NOT_FOUND");
    let hidden: HideCommentResponse = ctx.call(SVC, "HideComment", Some(&owner), &hide(&top.id)).await.unwrap();
    assert_eq!(hidden.comment.status, CommentStatus::COMMENT_STATUS_HIDDEN);
    assert_eq!(hidden.comment.hold_reason, HoldReason::HOLD_REASON_HIDDEN_BY_OWNER);
    assert!(!hidden.comment.pinned, "a hidden comment cannot stay pinned");

    // The thread left public view as a whole: the comment and its reply.
    assert_eq!(ctx.videos.counter(video.id, "comment_count"), 1);
    let removed = drain(&mut topic).await;
    assert!(removed.iter().any(|event| matches!(event, Pushed::CommentRemoved(gone) if gone.comment_id == top.id && gone.removed_count == 2)));
    let for_guests = ctx.comments(None, &video).await;
    assert_eq!((texts(&for_guests.comments), for_guests.total_count), (vec!["のこるコメント"], 1));
    let replies = ListRepliesRequest { comment_id: top.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason("CommentService", "ListReplies", None, &replies).await, "COMMENT_NOT_FOUND");
    assert_eq!(ctx.get_comment(None, &reply.id).await.unwrap_err().reason, "COMMENT_NOT_FOUND");
    // Its author still sees it, marked hidden; the reply's author keeps the reply but not the thread above it.
    assert_eq!(ctx.get_comment(Some(&bob), &top.id).await.unwrap().comment.status, CommentStatus::COMMENT_STATUS_HIDDEN);
    let orphan = ctx.get_comment(Some(&carol), &reply.id).await.unwrap();
    assert!(orphan.parent.as_option().is_none());
    // Nobody can add to a hidden thread.
    let add = CreateCommentRequest { parent_id: top.id.clone(), text: "まだ書ける?".into(), ..Default::default() };
    assert_eq!(ctx.reason("CommentService", "CreateComment", Some(&bob), &add).await, "COMMENT_NOT_PUBLISHED");
    assert_eq!(ctx.reason("CommentService", "CreateComment", Some(&carol), &add).await, "COMMENT_NOT_FOUND");
    // Nor give it a heart or a pin.
    let heart = SetCommentHeartRequest { comment_id: top.id.clone(), hearted: true, ..Default::default() };
    assert_eq!(ctx.reason("CommentService", "SetCommentHeart", Some(&owner), &heart).await, "COMMENT_NOT_PUBLISHED");
    assert_eq!(ctx.reason("CommentService", "PinComment", Some(&owner), &pin).await, "COMMENT_NOT_PINNABLE");

    // The owner finds it among the hidden comments and brings it back, replies and all.
    let listed = ctx.received(&owner, &received(CommentStatus::COMMENT_STATUS_HIDDEN)).await;
    assert_eq!(received_texts(&listed), ["かくされるスレッド"]);
    let _: ApproveCommentResponse = ctx.call(SVC, "ApproveComment", Some(&owner), &by_id(&top.id)).await.unwrap();
    assert_eq!(ctx.videos.counter(video.id, "comment_count"), 3);
    assert_eq!(ctx.comments(None, &video).await.total_count, 3);
    let back: ListRepliesResponse = ctx.call("CommentService", "ListReplies", None, &replies).await.unwrap();
    assert_eq!(ids(&back.replies), [reply.id.as_str()]);
    // Bringing it back is not news: the reply was announced when it was written.
    assert_eq!(ctx.notifications(bob.id, "engagement.reply").await.len(), 1);
}

#[tokio::test]
async fn a_blocked_commenter_is_not_told() {
    let ctx = start().await;
    let (owner, bob, carol) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await, ctx.app.user("Carol").await);
    let video = ctx.video(&owner, "v");
    let before = ctx.comment(&bob, &video, "ブロック前のコメント").await;
    ctx.comment(&carol, &video, "キャロルのコメント").await;
    assert_eq!(ctx.videos.counter(video.id, "comment_count"), 2);

    assert_eq!(ctx.reason(SVC, "BlockCommenter", None, &block_user(bob.id)).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.reason(SVC, "BlockCommenter", Some(&owner), &block_user(owner.id)).await, "CANNOT_BLOCK_SELF");
    assert_eq!(ctx.reason(SVC, "BlockCommenter", Some(&owner), &block_user(Uuid::now_v7())).await, "USER_NOT_FOUND");
    assert_eq!(ctx.reason(SVC, "BlockCommenter", Some(&owner), &BlockCommenterRequest::default()).await, "USER_NOT_FOUND");
    let blocked: BlockCommenterResponse = ctx.call(SVC, "BlockCommenter", Some(&owner), &block_user(bob.id)).await.unwrap();
    assert_eq!(blocked.blocked.commenter.user.id, bob.id.to_string());
    // Blocking again returns the same block.
    let again: BlockCommenterResponse = ctx.call(SVC, "BlockCommenter", Some(&owner), &block_user(bob.id)).await.unwrap();
    assert_eq!(again.blocked.id, blocked.blocked.id);

    // What bob wrote there is out of public view...
    assert_eq!(texts(&ctx.comments(None, &video).await.comments), ["キャロルのコメント"]);
    assert_eq!(ctx.videos.counter(video.id, "comment_count"), 1);
    // ...and what he writes from now on never enters it, without the owner hearing of it.
    let owner_notices = ctx.notification_kinds(owner.id).await.len();
    let after = ctx.comment(&bob, &video, "ブロック後のコメント").await;
    assert_eq!(ctx.comments(Some(&carol), &video).await.comments.len(), 1);
    assert_eq!(ctx.videos.counter(video.id, "comment_count"), 1);
    assert_eq!(ctx.notification_kinds(owner.id).await.len(), owner_notices);
    assert_eq!(ctx.get_comment(Some(&carol), &after.id).await.unwrap_err().reason, "COMMENT_NOT_FOUND");

    // To bob everything looks normal: his comments are there and read as published.
    assert_eq!(after.status, CommentStatus::COMMENT_STATUS_PUBLISHED);
    let his_view = ctx.comments(Some(&bob), &video).await;
    assert_eq!(texts(&his_view.comments), ["ブロック後のコメント", "キャロルのコメント", "ブロック前のコメント"]);
    assert!(his_view.comments.iter().all(|comment| comment.status == CommentStatus::COMMENT_STATUS_PUBLISHED));
    // He can even reply to himself; the reply stays in his private view too.
    let self_reply = ctx.reply(&bob, &after.id, "じぶんへの返信").await;
    assert_eq!(self_reply.status, CommentStatus::COMMENT_STATUS_PUBLISHED);
    assert_eq!(ctx.get_comment(None, &self_reply.id).await.unwrap_err().reason, "COMMENT_NOT_FOUND");

    // The owner's manager leaves a blocked commenter out, and lists the block.
    assert_eq!(received_texts(&ctx.received(&owner, &ListReceivedCommentsRequest::default()).await), ["キャロルのコメント"]);
    assert_eq!(ctx.reason(SVC, "ListBlockedCommenters", None, &ListBlockedCommentersRequest::default()).await, "SIGN_IN_REQUIRED");
    let blocks: ListBlockedCommentersResponse =
        ctx.call(SVC, "ListBlockedCommenters", Some(&owner), &ListBlockedCommentersRequest::default()).await.unwrap();
    assert_eq!(blocks.commenters.len(), 1);
    assert_eq!(blocks.commenters[0].commenter.user.handle, bob.handle);
    let none: ListBlockedCommentersResponse =
        ctx.call(SVC, "ListBlockedCommenters", Some(&bob), &ListBlockedCommentersRequest::default()).await.unwrap();
    assert!(none.commenters.is_empty());

    // Lifting the block: only the owner can, once; what was hidden stays
    // hidden until approved, and shows up in the manager again.
    let unblock = UnblockCommenterRequest { block_id: blocked.blocked.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "UnblockCommenter", None, &unblock).await, "SIGN_IN_REQUIRED");
    assert_eq!(ctx.reason(SVC, "UnblockCommenter", Some(&bob), &unblock).await, "BLOCK_NOT_FOUND");
    let _: UnblockCommenterResponse = ctx.call(SVC, "UnblockCommenter", Some(&owner), &unblock).await.unwrap();
    assert_eq!(ctx.reason(SVC, "UnblockCommenter", Some(&owner), &unblock).await, "BLOCK_NOT_FOUND");
    assert_eq!(ctx.comments(None, &video).await.comments.len(), 1);
    let hidden = ctx.received(&owner, &received(CommentStatus::COMMENT_STATUS_HIDDEN)).await;
    assert_eq!(hidden.comments.len(), 3);
    assert!(hidden.comments.iter().all(|entry| entry.comment.hold_reason == HoldReason::HOLD_REASON_BLOCKED_COMMENTER));
    let _: ApproveCommentResponse = ctx.call(SVC, "ApproveComment", Some(&owner), &by_id(&before.id)).await.unwrap();
    assert_eq!(ctx.comments(None, &video).await.comments.len(), 2);
    // New comments are public again.
    ctx.comment(&bob, &video, "ブロック解除後").await;
    assert_eq!(ctx.comments(None, &video).await.comments.len(), 3);
}

#[tokio::test]
async fn blocking_through_a_channel_comment_keeps_the_account_secret() {
    let ctx = start().await;
    let (owner, bob, stranger) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await, ctx.app.user("Stranger").await);
    let (video, elsewhere) = (ctx.video(&owner, "v"), ctx.video(&stranger, "elsewhere"));
    let channel = ctx.channels.add(&bob, "Bob Games");
    let as_channel = CreateCommentRequest { subject: video.subject_ref().into(), text: "チャンネルのコメント".into(), channel_id: channel.to_string(), ..Default::default() };
    let as_channel: CreateCommentResponse = ctx.call("CommentService", "CreateComment", Some(&bob), &as_channel).await.unwrap();
    let as_user = ctx.comment(&bob, &video, "本人のコメント").await;
    let on_other = ctx.comment(&bob, &elsewhere, "よその動画へ").await;

    // Only comments on one's own subjects can be used to block.
    assert_eq!(ctx.reason(SVC, "BlockCommenter", Some(&owner), &block_author_of(&on_other.id)).await, "NOT_SUBJECT_OWNER");
    assert_eq!(ctx.reason(SVC, "BlockCommenter", Some(&owner), &block_author_of(&Uuid::now_v7().to_string())).await, "COMMENT_NOT_FOUND");
    let own = ctx.comment(&owner, &video, "じぶんの").await;
    assert_eq!(ctx.reason(SVC, "BlockCommenter", Some(&owner), &block_author_of(&own.id)).await, "CANNOT_BLOCK_SELF");

    let blocked: BlockCommenterResponse =
        ctx.call(SVC, "BlockCommenter", Some(&owner), &block_author_of(&as_channel.comment.id)).await.unwrap();
    // The owner is shown the identity they blocked, never the account behind it.
    assert_eq!(blocked.blocked.commenter.channel.name, "Bob Games");
    assert!(blocked.blocked.commenter.user.as_option().is_none());
    // The comments written under that identity are hidden; the ones under the other are not...
    assert_eq!(texts(&ctx.comments(None, &video).await.comments), ["じぶんの", "本人のコメント"]);
    // ...but the block is on the account: whatever identity it uses next stays out of view.
    let next = ctx.comment(&bob, &video, "つぎのコメント").await;
    assert_eq!(next.status, CommentStatus::COMMENT_STATUS_PUBLISHED);
    assert_eq!(ctx.get_comment(None, &next.id).await.unwrap_err().reason, "COMMENT_NOT_FOUND");

    // Blocking the same account under its other identity is a second block;
    // nothing tells the owner that the two are one person.
    let second: BlockCommenterResponse = ctx.call(SVC, "BlockCommenter", Some(&owner), &block_author_of(&as_user.id)).await.unwrap();
    assert_ne!(second.blocked.id, blocked.blocked.id);
    assert_eq!(second.blocked.commenter.user.id, bob.id.to_string());
    let blocks: ListBlockedCommentersResponse =
        ctx.call(SVC, "ListBlockedCommenters", Some(&owner), &ListBlockedCommentersRequest { page: page(1, ""), ..Default::default() })
            .await
            .unwrap();
    assert_eq!(blocks.commenters.len(), 1);
    assert!(blocks.page.has_more);
    assert_eq!(blocks.commenters[0].id, second.blocked.id, "most recent first");
    let rest: ListBlockedCommentersResponse = ctx
        .call(
            SVC,
            "ListBlockedCommenters",
            Some(&owner),
            &ListBlockedCommentersRequest { page: page(1, &blocks.page.next_cursor), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(rest.commenters[0].id, blocked.blocked.id);
    assert!(!rest.page.has_more);
}

#[tokio::test]
async fn the_comment_manager_filters_what_was_received() {
    let ctx = start().await;
    let (owner, bob, carol, other_owner) =
        (ctx.app.user("Owner").await, ctx.app.user("Bob").await, ctx.app.user("Carol").await, ctx.app.user("Other").await);
    let (video, note, foreign) = (ctx.video(&owner, "動画"), ctx.note(&owner, "ノート"), ctx.video(&other_owner, "よその"));
    let channel = ctx.channels.add(&carol, "Carol TV");

    let b1 = ctx.comment(&bob, &video, "りんごが好き").await;
    let as_channel = CreateCommentRequest { subject: video.subject_ref().into(), text: "チャンネルから: 100%_本気".into(), channel_id: channel.to_string(), ..Default::default() };
    let c1: CreateCommentResponse = ctx.call("CommentService", "CreateComment", Some(&carol), &as_channel).await.unwrap();
    let c1 = c1.comment.as_option().unwrap().clone();
    let c2 = ctx.comment(&carol, &note, "ノートにAPPLEと書いた").await;
    let b2 = ctx.reply(&bob, &b1.id, "じぶんに返信").await;
    ctx.comment(&owner, &video, "オーナー自身のコメント").await;
    ctx.comment(&bob, &foreign, "よその動画へ").await;

    // Everything other people wrote on the caller's subjects, newest first, replies included.
    let all = ctx.received(&owner, &ListReceivedCommentsRequest::default()).await;
    assert_eq!(
        all.comments.iter().map(|entry| entry.comment.id.as_str()).collect::<Vec<_>>(),
        [b2.id.as_str(), c2.id.as_str(), c1.id.as_str(), b1.id.as_str()]
    );
    assert_eq!(all.comments[1].subject.title, "ノート");

    // By subject — only one's own.
    let on_note = ListReceivedCommentsRequest { subject: note.subject_ref().into(), ..Default::default() };
    assert_eq!(received_texts(&ctx.received(&owner, &on_note).await), ["ノートにAPPLEと書いた"]);
    let on_foreign = ListReceivedCommentsRequest { subject: foreign.subject_ref().into(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "ListReceivedComments", Some(&owner), &on_foreign).await, "NOT_SUBJECT_OWNER");

    // By text, case-insensitively, with pattern characters taken literally.
    let apple = ListReceivedCommentsRequest { query: "apple".into(), ..Default::default() };
    assert_eq!(received_texts(&ctx.received(&owner, &apple).await), ["ノートにAPPLEと書いた"]);
    let literal = ListReceivedCommentsRequest { query: "100%_".into(), ..Default::default() };
    assert_eq!(ctx.received(&owner, &literal).await.comments.len(), 1);
    let wildcard = ListReceivedCommentsRequest { query: "%".into(), ..Default::default() };
    assert_eq!(ctx.received(&owner, &wildcard).await.comments.len(), 1, "only the comment that contains a percent sign");

    // By identity: everything the author of a comment wrote under the same identity.
    let same_as = |id: &str| ListReceivedCommentsRequest { author_of_comment_id: id.to_owned(), ..Default::default() };
    assert_eq!(ctx.received(&owner, &same_as(&b1.id)).await.comments.len(), 2);
    assert_eq!(received_texts(&ctx.received(&owner, &same_as(&c1.id)).await), ["チャンネルから: 100%_本気"]);
    assert_eq!(received_texts(&ctx.received(&owner, &same_as(&c2.id)).await), ["ノートにAPPLEと書いた"]);
    assert_eq!(ctx.reason(SVC, "ListReceivedComments", Some(&owner), &same_as(&Uuid::now_v7().to_string())).await, "COMMENT_NOT_FOUND");

    // By kind of identity.
    let channels_only = ListReceivedCommentsRequest { author_kind: AuthorKind::AUTHOR_KIND_CHANNEL.into(), ..Default::default() };
    assert_eq!(ctx.received(&owner, &channels_only).await.comments.len(), 1);
    let users_only = ListReceivedCommentsRequest { author_kind: AuthorKind::AUTHOR_KIND_USER.into(), ..Default::default() };
    assert_eq!(ctx.received(&owner, &users_only).await.comments.len(), 3);

    // By time.
    let boundary = c2.created_at.as_option().unwrap().clone();
    let since = ListReceivedCommentsRequest { created_after: boundary.clone().into(), ..Default::default() };
    assert_eq!(ctx.received(&owner, &since).await.comments.len(), 2, "c2 and the reply after it");
    let until = ListReceivedCommentsRequest { created_before: boundary.into(), ..Default::default() };
    assert_eq!(ctx.received(&owner, &until).await.comments.len(), 2, "the two before c2");

    // In pages.
    let first = ctx.received(&owner, &ListReceivedCommentsRequest { page: page(3, ""), ..Default::default() }).await;
    assert_eq!(first.comments.len(), 3);
    assert!(first.page.has_more);
    let rest = ctx.received(&owner, &ListReceivedCommentsRequest { page: page(3, &first.page.next_cursor), ..Default::default() }).await;
    assert_eq!(rest.comments.iter().map(|entry| entry.comment.id.as_str()).collect::<Vec<_>>(), [b1.id.as_str()]);
    let bad = ListReceivedCommentsRequest { page: page(3, "???"), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "ListReceivedComments", Some(&owner), &bad).await, "INVALID_CURSOR");

    // People in a block with the owner drop out of the manager.
    ctx.block(&owner, &carol).await;
    let after_block = ctx.received(&owner, &ListReceivedCommentsRequest::default()).await;
    assert!(after_block.comments.iter().all(|entry| entry.comment.id != c1.id && entry.comment.id != c2.id));
}
