//! What the independent audit of the engagement domain found (issue #44):
//! each test failed before its fix.
mod common;

use std::time::Duration;

use chrono::{DateTime, Utc};

use common::*;
use tokuly_proto::{
    common::{SubjectRef, SubjectType},
    engagement::*,
};
use tokuly_testkit::TestUser;
use uuid::Uuid;

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

    /// What the moderation team was sent about a report: every version.
    async fn moderators_saw(&self, report: &Report) -> String {
        self.app
            .events("engagement.report.created")
            .await
            .into_iter()
            .find(|event| event.data["report_id"] == report.id.as_str())
            .map(|event| event.data["reported_content"].as_str().unwrap_or_default().to_owned())
            .expect("the report reached the moderators")
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
/// the way to report what was already received — but what was written
/// after the block never reached the other side, and across the block the
/// reporter is shown no text.
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
    assert_eq!(filed.subject_title, "コメント", "behind the block the reporter is shown no text");
    assert_eq!(ctx.moderators_saw(&filed).await, "ボブのばか");

    // The harasser blocks the victim first.
    let to_carol = ctx.comment(&alice, &video, "キャロルのばか").await;
    ctx.block(&alice, &carol).await;
    ctx.report_comment(&carol, &to_carol.id).await.expect("reportable after the harasser's block");
    // What the harasser wrote after the block never reached Carol.
    let later = ctx.comment(&alice, &video, "ブロックの後に書いた").await;
    assert_eq!(ctx.report_comment(&carol, &later.id).await.unwrap_err(), "SUBJECT_NOT_FOUND");
    assert_eq!(ctx.report_comment(&bob, &later.id).await.unwrap_err(), "SUBJECT_NOT_FOUND");

    // The owner of the content, blocked by the commenter.
    ctx.block(&alice, &owner).await;
    ctx.report_comment(&owner, &to_carol.id).await.expect("reportable by the owner after a block");
    // A block with the owner of the content counts too: after it, Dave
    // no longer saw the comments there.
    let dave = ctx.app.user("Dave").await;
    let video2 = ctx.video(&owner, "v2");
    ctx.block(&owner, &dave).await;
    let after = ctx.comment(&carol, &video2, "ブロックの後").await;
    assert_eq!(ctx.report_comment(&dave, &after.id).await.unwrap_err(), "SUBJECT_NOT_FOUND");
}

/// A comment taken out of public view stays reportable by those who saw it
/// while it was public, without showing them its text; a reply that was
/// never in public view (below a hidden comment) is not theirs to report.
#[tokio::test]
async fn a_hidden_comment_is_reported_without_its_text() {
    let ctx = start().await;
    let (owner, alice, bob) = (ctx.app.user("Owner").await, ctx.app.user("Alice").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    let comment = ctx.comment(&alice, &video, "ひどいことば").await;
    let hide = HideCommentRequest { comment_id: comment.id.clone(), ..Default::default() };
    let _: HideCommentResponse =
        ctx.call("CommentModerationService", "HideComment", Some(&owner), &hide).await.unwrap();
    let filed = ctx.report_comment(&bob, &comment.id).await.expect("Bob saw it while it was public");
    assert_eq!(filed.subject_title, "コメント");
    assert_eq!(ctx.moderators_saw(&filed).await, "ひどいことば");
    let mine: ListMyReportsResponse =
        ctx.call("ReportService", "ListMyReports", Some(&bob), &ListMyReportsRequest::default()).await.unwrap();
    assert_eq!(mine.reports[0].subject_title, "コメント");

    // Alice is blocked from commenting: her comment is hidden from the
    // start (she sees it as published). Once the block is lifted her reply
    // to it is published — below a comment nobody else can see.
    let block = BlockCommenterRequest {
        target: Some(tokuly_proto::engagement::__buffa::oneof::block_commenter_request::Target::UserId(
            alice.id.to_string(),
        )),
        ..Default::default()
    };
    let blocked: BlockCommenterResponse =
        ctx.call("CommentModerationService", "BlockCommenter", Some(&owner), &block).await.unwrap();
    let shadowed = ctx.comment(&alice, &video, "誰にも見えない").await;
    let lift = UnblockCommenterRequest { block_id: blocked.blocked.id.clone(), ..Default::default() };
    let _: UnblockCommenterResponse =
        ctx.call("CommentModerationService", "UnblockCommenter", Some(&owner), &lift).await.unwrap();
    let reply = ctx.reply(&alice, &shadowed.id, "これも").await;
    assert_eq!(reply.status, CommentStatus::COMMENT_STATUS_PUBLISHED);
    assert_eq!(ctx.report_comment(&bob, &shadowed.id).await.unwrap_err(), "SUBJECT_NOT_FOUND");
    assert_eq!(ctx.report_comment(&bob, &reply.id).await.unwrap_err(), "SUBJECT_NOT_FOUND");
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
    // The reporter is shown no text; the moderators get what was said.
    assert_eq!(filed.subject_title, "コメント");
    assert_eq!(filed.subject_path, format!("{}?comment={}", note.path, comment.id));
    assert_eq!(ctx.moderators_saw(&filed).await, "ひどいことば");
    let events = ctx.app.events_by("engagement.report.created", bob.id).await;
    assert_eq!(events[0].data["subject_owner_id"], serde_json::json!(alice.id));
    assert_eq!(events[0].data["subject_title"], "コメント");
    let stored: String = sqlx::query_scalar("SELECT subject_title FROM reports WHERE id = $1::uuid")
        .bind(&filed.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(stored, "コメント", "the reporter's copy keeps no text");
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

/// Editing a comment must not launder it: the moderators get every
/// version; the reporter is shown what the comment says now.
#[tokio::test]
async fn an_edited_comment_is_reported_with_every_version() {
    let ctx = start().await;
    let (owner, alice, bob) = (ctx.app.user("Owner").await, ctx.app.user("Alice").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    let comment = ctx.comment(&alice, &video, "ひどいことば").await;
    ctx.edit(&alice, &comment.id, "ごめんね").await.unwrap();
    let filed = ctx.report_comment(&bob, &comment.id).await.unwrap();
    assert_eq!(filed.subject_title, "ごめんね");
    assert_eq!(ctx.moderators_saw(&filed).await, "[2版] ひどいことば → ごめんね");

    // Edited, then deleted: every version is still there.
    ctx.delete(&alice, &comment.id).await;
    let filed = ctx.report_comment(&owner, &comment.id).await.unwrap();
    assert_eq!(filed.subject_title, "コメント");
    assert_eq!(ctx.moderators_saw(&filed).await, "[2版] ひどいことば → ごめんね");
}

/// Twenty harmless edits used to fill the cap, so the next hurtful version
/// was replaced by a harmless one without being kept. The latest edits are
/// kept now, on top of the first ones.
#[tokio::test]
async fn many_edits_do_not_launder_a_comment() {
    let ctx = start().await;
    let (owner, alice, bob) = (ctx.app.user("Owner").await, ctx.app.user("Alice").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    let comment = ctx.comment(&alice, &video, "はじめまして").await;
    for index in 1..=20 {
        ctx.edit(&alice, &comment.id, &format!("編集{index}")).await.unwrap();
    }
    ctx.edit(&alice, &comment.id, "ひどいことば").await.unwrap();
    ctx.edit(&alice, &comment.id, "なんでもない").await.unwrap();
    let filed = ctx.report_comment(&bob, &comment.id).await.unwrap();
    let seen = ctx.moderators_saw(&filed).await;
    assert!(seen.contains("ひどいことば → なんでもない"), "{seen}");
    assert!(seen.starts_with("[21版] はじめまして → 編集1 → "), "{seen}");
    assert!(seen.contains("（2件の編集は記録なし）"), "{seen}");
}

/// The moderation team gets what the reporter is no longer shown: the
/// admin case of a report about an edited, then deleted comment carries
/// every version, while its title is what the reporter saw.
#[tokio::test]
async fn the_moderation_case_carries_the_versions_of_a_withdrawn_comment() {
    let (ctx, admin) = start_with_admin().await;
    let (owner, alice, bob) = (ctx.app.user("Owner").await, ctx.app.user("Alice").await, ctx.app.user("Bob").await);
    let moderator = ctx.app.user_with_roles("Mod", &["moderator"]).await;
    let video = ctx.video(&owner, "v");
    let comment = ctx.comment(&alice, &video, "ひどいことば").await;
    ctx.edit(&alice, &comment.id, "ごめんね").await.unwrap();
    ctx.delete(&alice, &comment.id).await;
    let filed = ctx.report_comment(&bob, &comment.id).await.unwrap();
    assert_eq!(filed.subject_title, "コメント");

    let event = ctx
        .app
        .events("engagement.report.created")
        .await
        .into_iter()
        .find(|event| event.data["report_id"] == filed.id.as_str())
        .expect("the report was emitted");
    admin.handle_event(event).await.unwrap();
    let listed: tokuly_proto::admin::ListCasesResponse = ctx
        .app
        .call(
            "tokuly.admin.v1.ModerationService/ListCases",
            Some(&moderator),
            &tokuly_proto::admin::ListCasesRequest { owner_id: alice.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    let case = listed.cases.iter().find(|case| case.subject.id == comment.id).expect("a case about the comment");
    assert_eq!(case.reported_content, "", "lists carry no reported content; it travels with the reports");
    let opened: tokuly_proto::admin::GetCaseResponse = ctx
        .app
        .call(
            "tokuly.admin.v1.ModerationService/GetCase",
            Some(&moderator),
            &tokuly_proto::admin::GetCaseRequest { case_id: case.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(opened.moderation_case.subject_title, "コメント");
    assert_eq!(opened.moderation_case.reported_content, "[2版] ひどいことば → ごめんね");

    // An erased author takes it with them, like the title.
    admin
        .handle_event(tokuly_core::events::Event {
            id: uuid::Uuid::now_v7(),
            topic: "identity.user.deleted".into(),
            actor_id: None,
            occurred_at: chrono::Utc::now(),
            data: serde_json::json!({ "user_id": alice.id }),
        })
        .await
        .unwrap();
    let opened: tokuly_proto::admin::GetCaseResponse = ctx
        .app
        .call(
            "tokuly.admin.v1.ModerationService/GetCase",
            Some(&moderator),
            &tokuly_proto::admin::GetCaseRequest { case_id: case.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(opened.moderation_case.reported_content, "");
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
    let bob = ctx.app.user("Bob").await;
    let comment = ctx.comment(&alice, &video, "はじめまして").await;
    let announced = ctx.notifications(owner.id, "engagement.comment").await;
    assert_eq!(announced[0]["data"]["edited_at"], serde_json::Value::Null, "the version it quotes: unedited");
    ctx.edit(&alice, &comment.id, &format!("こんにちは @{}", bob.handle)).await.unwrap();
    let edited = ctx.app.events_by("engagement.comment.edited", alice.id).await;
    assert_eq!(edited.len(), 1);
    let edited = &edited[0].data;
    assert_eq!(edited["comment_id"], serde_json::json!(comment.id));
    assert_eq!(edited["subject_id"], serde_json::json!(video.id));
    // What notifications show of it now, and whom it mentions.
    assert_eq!(edited["body"]["ja"], format!("こんにちは @{}", bob.handle));
    assert_eq!(edited["mentioned"], serde_json::json!([bob.id]));
    // The mention the edit added names the version it quotes.
    let mention = ctx.notifications(bob.id, "engagement.mention").await;
    assert_eq!(mention.len(), 1);
    assert_eq!(mention[0]["data"]["edited_at"], edited["edited_at"]);
    assert_eq!(mention[0]["body"]["ja"], edited["body"]["ja"]);

    // Two edits are ordered by their version.
    ctx.edit(&alice, &comment.id, "またね").await.unwrap();
    let versions: Vec<DateTime<Utc>> = ctx
        .app
        .events_by("engagement.comment.edited", alice.id)
        .await
        .iter()
        .map(|event| serde_json::from_value(event.data["edited_at"].clone()).unwrap())
        .collect();
    assert!(versions[0] < versions[1], "{versions:?}");
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

/// Notify asked for it: a comment that leaves public view without being
/// deleted keeps its text in the inboxes of people who can no longer see
/// it, unless they hear of it.
#[tokio::test]
async fn hiding_a_comment_is_an_event_naming_who_still_sees_it() {
    let ctx = start().await;
    let (owner, alice, bob) = (ctx.app.user("Owner").await, ctx.app.user("Alice").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    let hidden_events = || async { ctx.events_about("engagement.comment.hidden", video.id).await };

    // The owner hides one: the owner still sees it.
    let comment = ctx.comment(&alice, &video, "はじめまして").await;
    let hide = HideCommentRequest { comment_id: comment.id.clone(), ..Default::default() };
    let _: HideCommentResponse =
        ctx.call("CommentModerationService", "HideComment", Some(&owner), &hide).await.unwrap();
    let hidden = hidden_events().await;
    assert_eq!(hidden.len(), 1);
    assert_eq!(hidden[0]["comment_id"], serde_json::json!(comment.id));
    assert_eq!(hidden[0]["author_id"], serde_json::json!(alice.id));
    assert_eq!(hidden[0]["visible_to"], serde_json::json!([owner.id]));
    assert_eq!(hidden[0]["status"], CommentStatus::COMMENT_STATUS_HIDDEN as i32);
    let hidden_at: DateTime<Utc> = serde_json::from_value(hidden[0]["changed_at"].clone()).unwrap();
    assert_eq!(ctx.app.events_by("engagement.comment.hidden", owner.id).await.len(), 1, "actor: the owner");

    // An edit that trips the owner's rules holds it for the owner's review.
    let held = ctx.comment(&bob, &video, "こんにちは").await;
    ctx.set_rules(&owner, false, false, &["ばか"]).await;
    ctx.edit(&bob, &held.id, "ばか").await.unwrap();
    let hidden = hidden_events().await;
    assert_eq!(hidden.len(), 2);
    assert_eq!(hidden[1]["comment_id"], serde_json::json!(held.id));
    assert_eq!(hidden[1]["status"], CommentStatus::COMMENT_STATUS_HELD as i32);
    assert_eq!(hidden[1]["visible_to"], serde_json::json!([owner.id]));
    let held_at: DateTime<Utc> = serde_json::from_value(hidden[1]["changed_at"].clone()).unwrap();
    assert!(hidden_at < held_at);

    // A block hides the rest of what Alice wrote there from everyone.
    let other = ctx.comment(&alice, &video, "またね").await;
    ctx.block(&owner, &alice).await;
    let hidden = hidden_events().await;
    let blocked: Vec<_> = hidden.iter().filter(|event| event["comment_id"] == serde_json::json!(other.id)).collect();
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0]["visible_to"], serde_json::json!([]));
    assert_eq!(blocked[0]["hold_reason"], HoldReason::HOLD_REASON_BLOCKED_COMMENTER as i32);
}

/// Video, stamp and search audit (#87) asked for it: a moderator working a
/// case sees reported content the way its reporters do, also after its
/// owner hid it from everyone else — and nowhere else.
#[tokio::test]
async fn moderators_resolve_a_reported_subject_like_its_reporters() {
    let ctx = start().await;
    let (owner, alice, bob, moderator) = (
        ctx.app.user("Owner").await,
        ctx.app.user("Alice").await,
        ctx.app.user("Bob").await,
        ctx.app.user("Mod").await,
    );
    let video = ctx.video(&owner, "ひどい動画");
    let comment = ctx.comment(&alice, &video, "ひどいコメント").await;
    let engagement = &ctx.engagement;
    let lookup = |staff: Uuid, kind: SubjectType, id: &str| {
        let id = Uuid::parse_str(id).unwrap();
        async move { engagement.resolve_reported_subject(staff, kind, id).await.unwrap() }
    };
    let video_id = video.id.to_string();
    // Nobody reported them yet.
    assert!(lookup(moderator.id, SubjectType::SUBJECT_TYPE_VIDEO, &video_id).await.is_none());

    let _: CreateReportResponse =
        ctx.call("ReportService", "CreateReport", Some(&bob), &report(video.subject_ref())).await.unwrap();
    ctx.report_comment(&bob, &comment.id).await.unwrap();
    // The owner hides the comment and makes the video private.
    let hide = HideCommentRequest { comment_id: comment.id.clone(), ..Default::default() };
    let _: HideCommentResponse =
        ctx.call("CommentModerationService", "HideComment", Some(&owner), &hide).await.unwrap();
    // (Made private, a video still resolves for a report by those who watched it.)
    ctx.videos.update(video.id, |subject| subject.withdrawn = true);
    // The normal lookup refuses both to the moderator.
    assert!(
        ctx.app.core.subjects.resolve(SubjectType::SUBJECT_TYPE_VIDEO, &video_id, Some(moderator.id)).await.is_err()
    );
    assert!(ctx.get_comment(Some(&moderator), &comment.id).await.is_err());

    // The case view resolves them as their reporter received them.
    let seen = lookup(moderator.id, SubjectType::SUBJECT_TYPE_VIDEO, &video_id).await.expect("the reported video");
    assert_eq!((seen.id, seen.owner_id, seen.title.as_str()), (video.id, owner.id, "ひどい動画"));
    let seen =
        lookup(moderator.id, SubjectType::SUBJECT_TYPE_COMMENT, &comment.id).await.expect("the reported comment");
    assert_eq!(seen.owner_id, alice.id);
    assert!(seen.title.contains("ひどいコメント"), "{}", seen.title);

    // Staff powers are not for one's own cases: not as the owner, not as the reporter.
    assert!(lookup(owner.id, SubjectType::SUBJECT_TYPE_VIDEO, &video_id).await.is_none());
    assert!(lookup(bob.id, SubjectType::SUBJECT_TYPE_VIDEO, &video_id).await.is_none());
    assert!(lookup(alice.id, SubjectType::SUBJECT_TYPE_COMMENT, &comment.id).await.is_none());
    // And nothing else answers like that: the moderator still cannot see,
    // comment on or react to either.
    assert!(ctx.get_comment(Some(&moderator), &comment.id).await.is_err());
    let request =
        SetReactionRequest { subject: video.subject_ref().into(), reaction: like().into(), ..Default::default() };
    assert!(
        ctx.call::<_, SetReactionResponse>("ReactionService", "SetReaction", Some(&moderator), &request).await.is_err()
    );
}

/// Review of #88: one notification per person, so an owner who was
/// mentioned in a top-level comment was told only by the mention; the edit
/// that drops it says what the owner's notification becomes.
#[tokio::test]
async fn an_edit_tells_what_the_owners_mention_becomes() {
    let ctx = start().await;
    let (owner, alice) = (ctx.app.user("Owner").await, ctx.app.user("Alice").await);
    let video = ctx.video(&owner, "今日のランチ");
    let comment = ctx.comment(&alice, &video, &format!("@{} みて", owner.handle)).await;
    assert_eq!(ctx.notification_kinds(owner.id).await, ["engagement.mention"]);
    ctx.edit(&alice, &comment.id, "みて").await.unwrap();
    let edited = ctx.app.events_by("engagement.comment.edited", alice.id).await;
    let notice = &edited[0].data["owner_notice"];
    assert_eq!(notice["user_id"], serde_json::json!(owner.id));
    assert_eq!(notice["kind"], "engagement.comment");
    assert_eq!(notice["title"]["ja"], "Aliceが「今日のランチ」にコメントしたよ!");

    // A reply is not the owner's to hear about.
    let reply = ctx.reply(&alice, &comment.id, "つづき").await;
    ctx.edit(&alice, &reply.id, "つづきだよ").await.unwrap();
    let edited = ctx.app.events_by("engagement.comment.edited", alice.id).await;
    assert!(edited[1].data["owner_notice"].is_null());
}

/// Review of #88: an edit that holds the comment again hides it before its
/// new text goes out, so consumers learn who may see it first.
#[tokio::test]
async fn an_edit_that_holds_the_comment_hides_it_first() {
    let ctx = start().await;
    let (owner, alice) = (ctx.app.user("Owner").await, ctx.app.user("Alice").await);
    let video = ctx.video(&owner, "v");
    let comment = ctx.comment(&alice, &video, "こんにちは").await;
    ctx.set_rules(&owner, false, false, &["ばか"]).await;
    ctx.edit(&alice, &comment.id, "ばか").await.unwrap();
    let topics: Vec<String> = ctx
        .app
        .events_by("engagement.comment.", alice.id)
        .await
        .into_iter()
        .filter(|event| event.data["comment_id"] == serde_json::json!(comment.id))
        .map(|event| event.topic)
        .collect();
    assert_eq!(topics, ["engagement.comment.published", "engagement.comment.hidden", "engagement.comment.edited"]);
}

/// Review of #88: a heart is for the reply's own author, who still sees the
/// reply when its thread is hidden: it names the thread, but not as a
/// `*_id` the notify domain withdraws by.
#[tokio::test]
async fn a_heart_on_a_reply_does_not_go_with_its_thread() {
    let ctx = start().await;
    let (owner, alice, bob) = (ctx.app.user("Owner").await, ctx.app.user("Alice").await, ctx.app.user("Bob").await);
    let video = ctx.video(&owner, "v");
    let thread = ctx.comment(&alice, &video, "スレッド").await;
    let reply = ctx.reply(&bob, &thread.id, "返信").await;
    let heart = SetCommentHeartRequest { comment_id: reply.id.clone(), hearted: true, ..Default::default() };
    let _: SetCommentHeartResponse = ctx.call("CommentService", "SetCommentHeart", Some(&owner), &heart).await.unwrap();
    let hearts = ctx.notifications(bob.id, "engagement.heart").await;
    assert_eq!(hearts.len(), 1);
    let data = hearts[0]["data"].as_object().unwrap();
    assert_eq!(data["comment_id"], serde_json::json!(reply.id));
    assert_eq!(data["thread"], serde_json::json!(thread.id));
    assert!(data.keys().filter(|key| key.ends_with("_id")).all(|key| key == "comment_id"), "{data:?}");
}

/// The server's bridge, as `bridges.rs` writes it.
struct ReportedForModerators(tokuly_engagement::Engagement);

#[async_trait::async_trait]
impl tokuly_admin::ReportedSubjects for ReportedForModerators {
    async fn reported_subject(
        &self,
        staff_id: Uuid,
        subject_type: SubjectType,
        subject_id: Uuid,
    ) -> tokuly_core::AppResult<Option<tokuly_core::subject::SubjectInfo>> {
        self.0.resolve_reported_subject(staff_id, subject_type, subject_id).await
    }
}

/// Through the real lookup: a case about a comment its subject's owner hid
/// after the report shows the comment to an uninvolved moderator.
#[tokio::test]
async fn the_case_view_shows_a_reported_comment_hidden_since() {
    let (ctx, admin) = start_with_admin().await;
    admin.set_reported_subjects(std::sync::Arc::new(ReportedForModerators(ctx.engagement.clone())));
    let (owner, alice, bob) = (ctx.app.user("Owner").await, ctx.app.user("Alice").await, ctx.app.user("Bob").await);
    let moderator = ctx.app.user_with_roles("Mod", &["moderator"]).await;
    let video = ctx.video(&owner, "v");
    let comment = ctx.comment(&alice, &video, "ひどいことば").await;
    let filed = ctx.report_comment(&bob, &comment.id).await.unwrap();
    let event = ctx
        .app
        .events("engagement.report.created")
        .await
        .into_iter()
        .find(|event| event.data["report_id"] == filed.id.as_str())
        .unwrap();
    admin.handle_event(event).await.unwrap();
    let hide = HideCommentRequest { comment_id: comment.id.clone(), ..Default::default() };
    let _: HideCommentResponse =
        ctx.call("CommentModerationService", "HideComment", Some(&owner), &hide).await.unwrap();

    let case_id: Uuid = sqlx::query_scalar("SELECT id FROM moderation_cases WHERE subject_id = $1::uuid")
        .bind(&comment.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    let open = |user: &TestUser| {
        let request = tokuly_proto::admin::GetCaseRequest { case_id: case_id.to_string(), ..Default::default() };
        let (app, user) = (&ctx.app, user.clone());
        async move {
            let opened: tokuly_proto::admin::GetCaseResponse =
                app.call("tokuly.admin.v1.ModerationService/GetCase", Some(&user), &request).await.unwrap();
            opened.moderation_case.current_subject.as_option().cloned()
        }
    };
    let current = open(&moderator).await.expect("the comment as its reporter saw it");
    assert!(current.title.contains("ひどいことば"), "{}", current.title);
}
