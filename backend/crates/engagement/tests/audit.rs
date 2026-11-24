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
        self.call::<_, CreateReportResponse>("ReportService", "CreateReport", Some(reporter), &report(comment_ref(comment_id)))
            .await
            .map(|response| response.report.as_option().expect("a report").clone())
            .map_err(|error| error.reason)
    }

    async fn edit(&self, author: &TestUser, comment_id: &str, text: &str) -> Result<Comment, String> {
        let request = UpdateCommentRequest { comment_id: comment_id.to_owned(), text: text.to_owned(), ..Default::default() };
        self.call::<_, UpdateCommentResponse>("CommentService", "UpdateComment", Some(author), &request)
            .await
            .map(|response| response.comment.as_option().expect("a comment").clone())
            .map_err(|error| error.reason)
    }

    async fn delete(&self, user: &TestUser, comment_id: &str) {
        let request = DeleteCommentRequest { comment_id: comment_id.to_owned(), ..Default::default() };
        let _: DeleteCommentResponse = self.call("CommentService", "DeleteComment", Some(user), &request).await.unwrap();
    }

    async fn unblock(&self, blocker: &TestUser, blocked: &TestUser) {
        let request = tokuly_proto::social::UnblockUserRequest { user_id: blocked.id.to_string(), ..Default::default() };
        let _: tokuly_proto::social::UnblockUserResponse =
            self.app.call("tokuly.social.v1.FriendService/UnblockUser", Some(blocker), &request).await.unwrap();
    }
}

// ───────────────────────────── reports ─────────────────────────────

// ───────────────────────────── comments ─────────────────────────────

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

// ───────────────────────────── counts ─────────────────────────────

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

// ───────────────────────────── load ─────────────────────────────

