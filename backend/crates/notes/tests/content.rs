//! What a note says: rich text, content warnings, attachments with alt text
//! and sensitive marks, and polls.
mod common;

use common::*;
use tokuly_proto::{
    common::Visibility,
    notes::{text_block::Block, text_inline::Inline, *},
};
use tokuly_testkit::TestUser;

fn poll_of(choices: &[&str], seconds: i64, results: PollResultsVisibility, multiple: bool) -> PollInput {
    PollInput {
        choices: choices.iter().map(|c| c.to_string()).collect(),
        multiple,
        duration_seconds: seconds,
        results_visibility: results.into(),
        ..Default::default()
    }
}

async fn vote(ctx: &Ctx, user: &TestUser, note: &Note, choices: &[i32]) -> Result<Poll, String> {
    let request = VotePollRequest { note_id: note.id.clone(), choices: choices.to_vec(), ..Default::default() };
    ctx.note::<_, VotePollResponse>("VotePoll", Some(user), &request)
        .await
        .map(|response| response.poll.as_option().cloned().expect("poll"))
        .map_err(|e| e.reason)
}

#[tokio::test]
async fn text_is_returned_as_a_tree_with_mentions_and_hashtags_resolved() {
    let ctx = start().await;
    let (alice, bob) = (ctx.member("alice").await, ctx.member("bob").await);
    let tag = unique("mfm");
    let text = format!("**やった** @{} #{tag} `#nottag` $[tada おめでとう] :nosuchemoji:\n> 引用", bob.handle);
    let note = ctx.post(&alice, &text).await;
    assert_eq!(note.text, text, "the text is kept as written");
    assert_eq!(note.hashtags, vec![tag.clone()], "code is not a hashtag");
    assert_eq!(note.language, "ja");
    let blocks = &note.body.blocks;
    assert_eq!(blocks.len(), 2);
    let Some(Block::Paragraph(paragraph)) = &blocks[0].block else { panic!("a paragraph first") };
    let mention = paragraph
        .inlines
        .iter()
        .find_map(|inline| match &inline.inline {
            Some(Inline::Mention(mention)) => Some(mention.clone()),
            _ => None,
        })
        .expect("a mention");
    assert_eq!((mention.handle.as_str(), mention.user_id.clone()), (bob.handle.as_str(), bob.id.to_string()));
    assert!(paragraph.inlines.iter().any(|i| matches!(&i.inline, Some(Inline::Effect(_)))));
    let emoji = paragraph
        .inlines
        .iter()
        .find_map(|inline| match &inline.inline {
            Some(Inline::Emoji(emoji)) => Some(emoji.clone()),
            _ => None,
        })
        .expect("an emoji node");
    assert!(emoji.image.as_option().is_none(), "an emoji that does not exist has no picture");
    assert!(matches!(blocks[1].block, Some(Block::Quote(_))));
    assert_eq!(ctx.notifications_for(bob.id, "notes.mention").await.len(), 1);
}

#[tokio::test]
async fn content_warnings_fold_and_attachments_carry_alt_text_and_sensitive_marks() {
    let ctx = start().await;
    let alice = ctx.member("alice").await;
    let images = [ctx.note_image(&alice).await, ctx.note_image(&alice).await];
    let note = ctx
        .post_with(&alice, "結末は…", |request| {
            request.content_warning_text = "映画のネタバレ".into();
            request.asset_ids = images.to_vec();
            request.attachment_details = vec![AttachmentDetailsInput {
                asset_id: images[1].clone(),
                alt_text: "夕焼けの海".into(),
                sensitive: true,
                ..Default::default()
            }];
        })
        .await;
    assert!(note.content_warning, "a warning line folds the note");
    assert_eq!(note.content_warning_text, "映画のネタバレ");
    let details: Vec<(&str, &str, bool)> =
        note.attachment_details.iter().map(|d| (d.asset_id.as_str(), d.alt_text.as_str(), d.sensitive)).collect();
    assert_eq!(details, vec![(images[0].as_str(), "", false), (images[1].as_str(), "夕焼けの海", true)]);
    let reason = ctx.post_reason(&alice, "x", |request| request.content_warning_text = "注".repeat(101)).await;
    assert_eq!(reason, "CW_TOO_LONG");
    let image = ctx.note_image(&alice).await;
    let reason = ctx
        .post_reason(&alice, "x", |request| {
            request.asset_ids = vec![image.clone()];
            request.attachment_details = vec![AttachmentDetailsInput {
                asset_id: image.clone(),
                alt_text: "a".repeat(1501),
                ..Default::default()
            }];
        })
        .await;
    assert_eq!(reason, "ALT_TEXT_TOO_LONG");
    // Ten images fit.
    let mut ten = Vec::new();
    for _ in 0..10 {
        ten.push(ctx.note_image(&alice).await);
    }
    assert_eq!(ctx.post_with(&alice, "", |request| request.asset_ids = ten.clone()).await.attachments.len(), 10);
}

#[tokio::test]
async fn polls_count_votes_privately_and_show_results_when_allowed() {
    let ctx = start().await;
    let (alice, bob, carol, dave) =
        (ctx.member("alice").await, ctx.member("bob").await, ctx.member("carol").await, ctx.member("dave").await);
    let note = ctx
        .post_with(&alice, "どっち派?", |request| {
            request.poll =
                poll_of(&["いぬ", "ねこ"], 3600, PollResultsVisibility::POLL_RESULTS_VISIBILITY_AFTER_VOTING, false)
                    .into();
        })
        .await;
    let poll = note.poll.as_option().expect("a poll");
    assert_eq!(poll.choices.len(), 2);
    assert!(poll.results_visible, "the author sees the counts");
    let before = ctx.get_note(Some(&bob), &note.id).await.unwrap();
    assert!(!before.poll.results_visible && before.poll.voter_count == 0, "nothing before voting");

    let mut listener = ctx.app.listen_topic(&tokuly_notes::model::note_topic(id(&note.id))).await;
    let voted = vote(&ctx, &bob, &note, &[1]).await.unwrap();
    assert!(voted.results_visible);
    assert_eq!((voted.choices[1].votes, voted.voter_count, voted.my_choices.clone()), (1, 1, vec![1]));
    assert!(matches!(next_push(&mut listener).await, Some(tokuly_proto::notes::notes_event::Event::PollUpdated(_))));
    assert_eq!(vote(&ctx, &bob, &note, &[0]).await.unwrap_err(), "ALREADY_VOTED", "a vote is final");
    assert_eq!(vote(&ctx, &carol, &note, &[0, 1]).await.unwrap_err(), "INVALID_CHOICE", "one choice only");
    assert_eq!(vote(&ctx, &carol, &note, &[5]).await.unwrap_err(), "INVALID_CHOICE");

    // Two people vote at the same moment: both count.
    let (c, d) = tokio::join!(vote(&ctx, &carol, &note, &[0]), vote(&ctx, &dave, &note, &[0]));
    assert!(c.is_ok() && d.is_ok());
    let now = ctx.get_note(Some(&dave), &note.id).await.unwrap();
    assert_eq!((now.poll.choices[0].votes, now.poll.voter_count), (2, 3));

    // Nobody learns who voted for what: not even the author.
    let as_author = ctx.get_note(Some(&alice), &note.id).await.unwrap();
    assert!(as_author.poll.my_choices.is_empty());

    // A plain note has no poll; a friends-only poll is not for strangers.
    let plain = ctx.post(&alice, "no poll").await;
    assert_eq!(vote(&ctx, &bob, &plain, &[0]).await.unwrap_err(), "POLL_NOT_FOUND");
    let hidden = ctx
        .post_with(&alice, "ともだちだけ", |request| {
            request.visibility = Visibility::VISIBILITY_FRIENDS.into();
            request.poll =
                poll_of(&["a", "b"], 3600, PollResultsVisibility::POLL_RESULTS_VISIBILITY_ALWAYS, false).into();
        })
        .await;
    assert_eq!(vote(&ctx, &bob, &hidden, &[0]).await.unwrap_err(), "NOTE_NOT_FOUND");

    // It ends: voters and the author are told once, and nobody votes any more.
    sqlx::query("UPDATE note_polls SET ends_at = now() - interval '1 second' WHERE note_id = $1")
        .bind(id(&note.id))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert!(ctx.notes.close_polls().await.unwrap() >= 1);
    ctx.notes.close_polls().await.unwrap();
    for person in [&bob, &carol, &dave, &alice] {
        assert_eq!(ctx.notifications_for(person.id, "notes.poll_ended").await.len(), 1, "{}", person.handle);
    }
    let eve = ctx.member("eve").await;
    assert_eq!(vote(&ctx, &eve, &note, &[0]).await.unwrap_err(), "POLL_CLOSED");
    let after = ctx.get_note(Some(&eve), &note.id).await.unwrap();
    assert!(after.poll.closed && after.poll.results_visible, "everyone sees the result once it ended");

    // Counts do not move when a voter's account goes away.
    ctx.notes
        .handle_event(tokuly_core::events::Event {
            id: uuid::Uuid::now_v7(),
            topic: "identity.user.deleted".into(),
            actor_id: None,
            occurred_at: chrono::Utc::now(),
            data: serde_json::json!({ "user_id": bob.id }),
        })
        .await
        .unwrap();
    assert_eq!(ctx.get_note(Some(&eve), &note.id).await.unwrap().poll.voter_count, 3);

    // Multiple choice, results after the end only.
    let multi = ctx
        .post_with(&alice, "好きなの全部", |request| {
            request.poll =
                poll_of(&["a", "b", "c"], 600, PollResultsVisibility::POLL_RESULTS_VISIBILITY_AFTER_END, true).into();
        })
        .await;
    let mine = vote(&ctx, &eve, &multi, &[0, 2]).await.unwrap();
    assert_eq!(mine.my_choices, vec![0, 2]);
    assert!(!mine.results_visible);
    let reason = ctx
        .post_reason(&alice, "?", |request| request.poll = poll_of(&["only"], 600, Default::default(), false).into())
        .await;
    assert_eq!(reason, "INVALID_POLL");
    let empty_text_poll = ctx
        .post_with(&alice, "", |request| request.poll = poll_of(&["a", "b"], 600, Default::default(), false).into())
        .await;
    assert!(empty_text_poll.poll.as_option().is_some(), "a poll alone is a note");
}
