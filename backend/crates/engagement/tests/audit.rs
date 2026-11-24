//! What the independent audit of the engagement domain found (issue #44):
//! each test failed before its fix.
mod common;

use std::time::Duration;

use common::*;
use tokuly_proto::{
    common::{SubjectRef, SubjectType},
    engagement::*,
};
use tokuly_testkit::TestUser;

fn report(subject: SubjectRef) -> CreateReportRequest {
    CreateReportRequest {
        subject: subject.into(),
        category: ReportCategory::REPORT_CATEGORY_HARASSMENT.into(),
        ..Default::default()
    }
}

impl Ctx {
    async fn report_comment(&self, reporter: &TestUser, comment_id: &str) -> Result<Report, String> {
        self.call::<_, CreateReportResponse>(
            "ReportService",
            "CreateReport",
            Some(reporter),
            &report(comment_ref(comment_id)),
        )
        .await
        .map(|response| response.report.as_option().expect("a report").clone())
        .map_err(|error| error.reason)
    }

    async fn edit(&self, author: &TestUser, comment_id: &str, text: &str) -> Result<Comment, String> {
        let request =
            UpdateCommentRequest { comment_id: comment_id.to_owned(), text: text.to_owned(), ..Default::default() };
        self.call::<_, UpdateCommentResponse>("CommentService", "UpdateComment", Some(author), &request)
            .await
            .map(|response| response.comment.as_option().expect("a comment").clone())
            .map_err(|error| error.reason)
    }

    async fn delete(&self, user: &TestUser, comment_id: &str) {
        let request = DeleteCommentRequest { comment_id: comment_id.to_owned(), ..Default::default() };
        let _: DeleteCommentResponse =
            self.call("CommentService", "DeleteComment", Some(user), &request).await.unwrap();
    }

    async fn unblock(&self, blocker: &TestUser, blocked: &TestUser) {
        let request =
            tokuly_proto::social::UnblockUserRequest { user_id: blocked.id.to_string(), ..Default::default() };
        let _: tokuly_proto::social::UnblockUserResponse =
            self.app.call("tokuly.social.v1.FriendService/UnblockUser", Some(blocker), &request).await.unwrap();
    }
}

// ───────────────────────────── reports ─────────────────────────────

/// A block hides comments in both directions; it must not also take away
/// the way to report what was already received.
#[tokio::test]
async fn a_comment_stays_reportable_across_a_block() {
    let ctx = start().await;
    let (owner, alice, bob, carol) = (
        ctx.app.user("Owner").await,
        ctx.app.user("Alice").await,
        ctx.app.user("Bob").await,
        ctx.app.user("Carol").await,
    );
    let video = ctx.video(&owner, "v");

    // The victim blocks the harasser, then reports.
    let to_bob = ctx.comment(&alice, &video, "ボブのばか").await;
    ctx.block(&bob, &alice).await;
    let filed = ctx.report_comment(&bob, &to_bob.id).await.expect("reportable after the victim's block");
    assert_eq!(filed.subject_title, "ボブのばか");

    // The harasser blocks the victim first.
    let to_carol = ctx.comment(&alice, &video, "キャロルのばか").await;
    ctx.block(&alice, &carol).await;
    ctx.report_comment(&carol, &to_carol.id).await.expect("reportable after the harasser's block");

    // The owner of the content, blocked by the commenter.
    ctx.block(&alice, &owner).await;
    ctx.report_comment(&owner, &to_carol.id).await.expect("reportable by the owner after a block");
}

/// Deleting a comment must not take away the report of those who saw it,
/// and must not open it to those who never could.
#[tokio::test]
async fn a_deleted_comment_stays_reportable_by_those_who_received_it() {
    let ctx = start().await;
    let (owner, alice, bob, carol) = (
        ctx.app.user("Owner").await,
        ctx.app.user("Alice").await,
        ctx.app.user("Bob").await,
        ctx.app.user("Carol").await,
    );
    let note = ctx.note(&owner, "友達だけ");
    ctx.notes.update(note.id, |note| note.audience = Audience::Only(vec![alice.id, bob.id]));

    let comment = ctx.comment(&alice, &note, "ひどいことば").await;
    ctx.delete(&alice, &comment.id).await;
    let filed = ctx.report_comment(&bob, &comment.id).await.expect("reportable after its deletion");
    assert_eq!(filed.subject_title, "ひどいことば");
    assert_eq!(filed.subject_path, format!("{}?comment={}", note.path, comment.id));
    let events = ctx.app.events_by("engagement.report.created", bob.id).await;
    assert_eq!(events[0].data["subject_owner_id"], serde_json::json!(alice.id));
    // Someone who could never see the note learns nothing.
    assert_eq!(ctx.report_comment(&carol, &comment.id).await.unwrap_err(), "SUBJECT_NOT_FOUND");
    // Nor can the author report their own words.
    assert_eq!(ctx.report_comment(&alice, &comment.id).await.unwrap_err(), "CANNOT_REPORT_OWN");

    // A comment that never left the review queue was only ever the owner's to see.
    ctx.set_rules(&owner, true, false, &[]).await;
    let held = ctx.comment(&alice, &note, "保留されたひどいことば").await;
    assert_eq!(held.status, CommentStatus::COMMENT_STATUS_HELD);
    ctx.delete(&alice, &held.id).await;
    assert_eq!(ctx.report_comment(&bob, &held.id).await.unwrap_err(), "SUBJECT_NOT_FOUND");
    ctx.report_comment(&owner, &held.id).await.expect("the owner saw it in the review queue");

    // A takedown forgets what was kept.
    ctx.deliver(
        "admin.content.removed",
        None,
        serde_json::json!({ "subject_type": SubjectType::SUBJECT_TYPE_COMMENT as i32, "subject_id": comment.id }),
    )
    .await;
    assert_eq!(ctx.report_comment(&owner, &comment.id).await.unwrap_err(), "SUBJECT_NOT_FOUND");
}

/// Editing a comment must not launder it: the report carries every version.
#[tokio::test]
async fn an_edited_comment_is_reported_with_every_version() {
    let ctx = start().await;
    let (owner, alice, bob) = (ctx.app.user("Owner").await, ctx.app.user("Alice").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    let comment = ctx.comment(&alice, &video, "ひどいことば").await;
    ctx.edit(&alice, &comment.id, "ごめんね").await.unwrap();
    let filed = ctx.report_comment(&bob, &comment.id).await.unwrap();
    assert_eq!(filed.subject_title, "[2版] ひどいことば → ごめんね");

    // Edited, then deleted: every version is still there.
    ctx.delete(&alice, &comment.id).await;
    let filed = ctx.report_comment(&owner, &comment.id).await.unwrap();
    assert_eq!(filed.subject_title, "[2版] ひどいことば → ごめんね");
}

/// What was kept for reports goes 30 days after its removal — counted from
/// the removal, not from when the comment was written — and at once when
/// its author's account is erased.
#[tokio::test]
async fn kept_texts_expire_from_their_removal_and_with_the_account() {
    let ctx = start().await;
    let (owner, alice, bob) = (ctx.app.user("Owner").await, ctx.app.user("Alice").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    let old = ctx.comment(&alice, &video, "昔のひどいことば").await;
    // Written long ago, deleted now: still reportable.
    sqlx::query("UPDATE comments SET created_at = now() - interval '90 days' WHERE id = $1::uuid")
        .bind(&old.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.delete(&alice, &old.id).await;
    ctx.engagement.purge_removed_comment_texts().await.unwrap();
    ctx.report_comment(&bob, &old.id).await.expect("kept from its deletion on");
    // Deleted 31 days ago: gone.
    sqlx::query("UPDATE comment_removed_texts SET removed_at = now() - interval '31 days' WHERE comment_id = $1::uuid")
        .bind(&old.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.engagement.purge_removed_comment_texts().await.unwrap();
    assert_eq!(ctx.report_comment(&owner, &old.id).await.unwrap_err(), "SUBJECT_NOT_FOUND");

    let edited = ctx.comment(&alice, &video, "ひどいことば").await;
    ctx.edit(&alice, &edited.id, "ごめんね").await.unwrap();
    ctx.deliver("identity.user.deleted", None, serde_json::json!({ "user_id": alice.id })).await;
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM comment_removed_texts WHERE author_id = $1")
        .bind(alice.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

// ───────────────────────────── comments ─────────────────────────────

/// Notify asked for it: an inbox entry about a comment keeps its old text
/// unless it hears of the edit.
#[tokio::test]
async fn an_edit_is_an_event() {
    let ctx = start().await;
    let (owner, alice) = (ctx.app.user("Owner").await, ctx.app.user("Alice").await);
    let video = ctx.video(&owner, "v");
    let comment = ctx.comment(&alice, &video, "はじめまして").await;
    ctx.edit(&alice, &comment.id, "こんにちは").await.unwrap();
    let edited = ctx.app.events_by("engagement.comment.edited", alice.id).await;
    assert_eq!(edited.len(), 1);
    assert_eq!(edited[0].data["comment_id"], serde_json::json!(comment.id));
    assert_eq!(edited[0].data["subject_id"], serde_json::json!(video.id));
}

/// A commenter who blocks the owner of the content must not leave their
/// comments up there, where the owner can no longer see or remove them.
#[tokio::test]
async fn a_commenter_who_blocks_the_owner_takes_their_comments_down() {
    let ctx = start().await;
    let (owner, alice, carol) = (ctx.app.user("Owner").await, ctx.app.user("Alice").await, ctx.app.user("Carol").await);
    let video = ctx.video(&owner, "v");
    let comment = ctx.comment(&alice, &video, "オーナーの悪口").await;
    ctx.block(&alice, &owner).await;
    let seen = ctx.comments(Some(&carol), &video).await;
    assert!(!ids(&seen.comments).contains(&comment.id.as_str()), "the comment stayed up for everyone else");
    assert_eq!(seen.total_count, 0);
    // The commenter is not told.
    let mine: ListMyCommentsResponse =
        ctx.call("CommentService", "ListMyComments", Some(&alice), &ListMyCommentsRequest::default()).await.unwrap();
    assert_eq!(mine.comments[0].comment.status, CommentStatus::COMMENT_STATUS_PUBLISHED);
}

/// A block hides everything, also for a prolific commenter, in batches of
/// 200 per transaction.
#[tokio::test]
async fn a_block_hides_every_comment_in_batches() {
    let ctx = start().await;
    let (owner, alice) = (ctx.app.user("Owner").await, ctx.app.user("Alice").await);
    let video = ctx.video(&owner, "v");
    let written = sqlx::query(
        "INSERT INTO comments (subject_type, subject_id, subject_owner_id, author_id, text)
         SELECT $1, $2, $3, $4, 'こんにちは ' || n FROM generate_series(1, 450) AS n",
    )
    .bind(SubjectType::SUBJECT_TYPE_VIDEO as i32)
    .bind(video.id)
    .bind(owner.id)
    .bind(alice.id)
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(written.rows_affected(), 450);
    ctx.block(&owner, &alice).await;
    let published: i64 = sqlx::query_scalar("SELECT count(*) FROM comments WHERE author_id = $1 AND status <> 3")
        .bind(alice.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(published, 0);
}

/// An event delivered again after the block was lifted must not undo the
/// owner's decision to bring the comments back.
#[tokio::test]
async fn a_late_block_event_does_not_hide_comments_again() {
    let ctx = start().await;
    let (owner, alice, carol) = (ctx.app.user("Owner").await, ctx.app.user("Alice").await, ctx.app.user("Carol").await);
    let video = ctx.video(&owner, "v");
    let comment = ctx.comment(&alice, &video, "やあ").await;
    ctx.block(&owner, &alice).await;
    ctx.unblock(&owner, &alice).await;
    let approve = ApproveCommentRequest { comment_id: comment.id.clone(), ..Default::default() };
    let _: ApproveCommentResponse =
        ctx.call("CommentModerationService", "ApproveComment", Some(&owner), &approve).await.unwrap();
    assert_eq!(ctx.comments(Some(&carol), &video).await.total_count, 1);

    ctx.deliver(
        "social.user.blocked",
        Some(owner.id),
        serde_json::json!({ "user_id": owner.id, "blocked_id": alice.id }),
    )
    .await;
    assert_eq!(ctx.comments(Some(&carol), &video).await.total_count, 1);
}

/// Mentioning, unmentioning and mentioning again by editing is one
/// notification, not one per edit.
#[tokio::test]
async fn editing_a_mention_in_and_out_notifies_once() {
    let ctx = start().await;
    let (owner, alice, bob) = (ctx.app.user("Owner").await, ctx.app.user("Alice").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    let comment = ctx.comment(&alice, &video, &format!("やあ @{}", bob.handle)).await;
    for round in 0..3 {
        ctx.edit(&alice, &comment.id, &format!("やあ{round}")).await.unwrap();
        ctx.edit(&alice, &comment.id, &format!("やあ{round} @{}", bob.handle)).await.unwrap();
    }
    assert_eq!(ctx.notifications(bob.id, "engagement.mention").await.len(), 1);
}

/// While a subject does not accept comments, nothing written there changes.
#[tokio::test]
async fn comments_cannot_be_edited_while_they_are_turned_off() {
    let ctx = start().await;
    let (owner, alice, bob) = (ctx.app.user("Owner").await, ctx.app.user("Alice").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    let comment = ctx.comment(&alice, &video, "やあ").await;
    ctx.videos.update(video.id, |video| video.interactions_enabled = false);
    assert_eq!(ctx.edit(&alice, &comment.id, &format!("やあ @{}", bob.handle)).await.unwrap_err(), "COMMENTS_DISABLED");
    assert!(ctx.notifications(bob.id, "engagement.mention").await.is_empty());
}

// ───────────────────────────── counts ─────────────────────────────

/// Taking back a reaction or a bookmark works by UUID on anything; the
/// answer must not tell the numbers of a subject the caller cannot see.
#[tokio::test]
async fn taking_something_back_tells_nothing_about_unseen_subjects() {
    let ctx = start().await;
    let (owner, bob, mallory) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await, ctx.app.user("Mallory").await);
    let note = ctx.note(&owner, "ひみつ");
    ctx.notes.update(note.id, |note| note.audience = Audience::Only(vec![bob.id]));
    ctx.react(&bob, note.subject_ref(), like()).await;
    let save = AddBookmarkRequest { subject: note.subject_ref().into(), ..Default::default() };
    let _: AddBookmarkResponse = ctx.call("BookmarkService", "AddBookmark", Some(&bob), &save).await.unwrap();

    let unreact = RemoveReactionRequest { subject: note.subject_ref().into(), ..Default::default() };
    let answer: RemoveReactionResponse =
        ctx.call("ReactionService", "RemoveReaction", Some(&mallory), &unreact).await.unwrap();
    assert_eq!(answer.reactions.like_count, 0, "the like count of a note Mallory cannot see");
    let unsave = RemoveBookmarkRequest { subject: note.subject_ref().into(), ..Default::default() };
    let answer: RemoveBookmarkResponse =
        ctx.call("BookmarkService", "RemoveBookmark", Some(&mallory), &unsave).await.unwrap();
    assert_eq!(answer.bookmark_count, 0, "the bookmark count of a note Mallory cannot see");

    // Bob, who can, still gets the numbers.
    let answer: RemoveBookmarkResponse =
        ctx.call("BookmarkService", "RemoveBookmark", Some(&bob), &unsave).await.unwrap();
    assert_eq!(answer.bookmark_count, 0);
    let save_again: AddBookmarkResponse = ctx.call("BookmarkService", "AddBookmark", Some(&bob), &save).await.unwrap();
    assert_eq!(save_again.bookmark_count, 1);
}

/// One account sharing the same thing over and over is one share.
#[tokio::test]
async fn sharing_the_same_thing_again_does_not_count_again() {
    let ctx = start().await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    let share = |target: ShareTarget| RecordShareRequest {
        subject: video.subject_ref().into(),
        target: target.into(),
        ..Default::default()
    };
    let mut last = 0;
    for _ in 0..5 {
        let answer: RecordShareResponse = ctx
            .call("EngagementService", "RecordShare", Some(&bob), &share(ShareTarget::SHARE_TARGET_LINK_COPY))
            .await
            .unwrap();
        assert!(!answer.url.is_empty());
        last = answer.share_count;
    }
    assert_eq!(last, 1);
    let answer: RecordShareResponse =
        ctx.call("EngagementService", "RecordShare", Some(&bob), &share(ShareTarget::SHARE_TARGET_CHAT)).await.unwrap();
    assert_eq!(answer.share_count, 2);
    assert_eq!(ctx.events_about("engagement.share.recorded", video.id).await.len(), 2);
}

// ───────────────────────────── load ─────────────────────────────

/// Many concurrent calls of one account at the RPCs that open
/// transactions or take locks finish well within the pool's acquire
/// timeout: nothing waits for a second connection while holding one.
#[tokio::test]
async fn one_account_cannot_starve_the_pool() {
    let ctx = start().await;
    let (owner, bob) = (ctx.app.user("Owner").await, ctx.app.user("Bob").await);
    let videos: Vec<_> = (0..4).map(|i| ctx.video(&owner, &format!("v{i}"))).collect();
    let thread = ctx.comment(&owner, &videos[0], "スレッド").await;
    let calls = videos.iter().flat_map(|video| {
        let subject = video.subject_ref();
        let ctx = &ctx;
        let bob = &bob;
        let thread_id = thread.id.clone();
        [
            Box::pin(async move {
                let request = SetReactionRequest {
                    subject: subject.clone().into(),
                    reaction: like().into(),
                    ..Default::default()
                };
                ctx.call::<_, serde_json::Value>("ReactionService", "SetReaction", Some(bob), &request).await
            }) as std::pin::Pin<Box<dyn std::future::Future<Output = _>>>,
            Box::pin(async move {
                let request = AddBookmarkRequest { subject: video.subject_ref().into(), ..Default::default() };
                ctx.call::<_, serde_json::Value>("BookmarkService", "AddBookmark", Some(bob), &request).await
            }),
            Box::pin(async move {
                let request = CreateCommentRequest {
                    subject: video.subject_ref().into(),
                    text: format!("こんにちは {}", video.id),
                    ..Default::default()
                };
                ctx.call::<_, serde_json::Value>("CommentService", "CreateComment", Some(bob), &request).await
            }),
            Box::pin(async move {
                let request = CreateCommentRequest {
                    parent_id: thread_id,
                    text: format!("返信 {}", video.id),
                    ..Default::default()
                };
                ctx.call::<_, serde_json::Value>("CommentService", "CreateComment", Some(bob), &request).await
            }),
            Box::pin(async move {
                ctx.call::<_, serde_json::Value>(
                    "ReportService",
                    "CreateReport",
                    Some(bob),
                    &report(video.subject_ref()),
                )
                .await
            }),
        ]
    });
    let started = std::time::Instant::now();
    let results = futures::future::join_all(calls).await;
    for result in &results {
        assert!(result.is_ok(), "{result:?}");
    }
    assert!(started.elapsed() < Duration::from_secs(8), "took {:?}", started.elapsed());
}
