//! The anonymity properties of the question box.
//!
//! - Nothing the recipient is sent names an anonymous sender, or carries
//!   anything derived from them.
//! - A sender the recipient blocked cannot tell: every answer they get is
//!   the one anyone would get.
//! - Blocking tells the recipient nothing either.
//! - Moderators, and only they, can see who sent something.
mod common;

use common::*;
use tokuly_core::events::Event;
use tokuly_media::testing::ready_image;
use tokuly_proto::{
    ask::{ask_event::Event as Push, *},
    common::SubjectType,
    media::AssetPurpose,
};
use tokuly_testkit::TestUser;
use uuid::Uuid;

/// A name no response can contain by accident.
const SENDER: &str = "Zebedee Quillfeather";

fn question_id(id: &str) -> GetQuestionRequest {
    GetQuestionRequest { question_id: id.to_owned(), ..Default::default() }
}

/// What a sender is told about a question they sent, without the parts that
/// differ between any two questions.
fn told(mut question: serde_json::Value) -> serde_json::Value {
    for key in ["id", "createdAt", "path"] {
        question.as_object_mut().expect("a question").remove(key);
    }
    question
}

/// The events one request wrote, as far as their shape goes: topic, actor
/// and the names of the payload's fields.
fn shape(events: &[Event]) -> Vec<(String, Option<Uuid>, Vec<String>)> {
    events
        .iter()
        .map(|e| {
            (e.topic.clone(), e.actor_id, e.data.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default())
        })
        .collect()
}

#[tokio::test]
async fn nothing_the_recipient_is_sent_names_an_anonymous_sender() {
    let ctx = start().await;
    let (owner, sender, reader) = (ctx.user("Owner").await, ctx.user(SENDER).await, ctx.user("Reader").await);
    ctx.open_box(&owner, true).await;
    let mut pushes = ctx.app.listen_user(owner.id).await;

    let sent = ctx.try_ask(&sender, &owner, "好きな食べ物は?", false).await.unwrap();
    let events = ctx.deliver().await;
    assert!(!events.is_empty());
    for event in &events {
        assert_ne!(event.actor_id, Some(sender.id), "{} names the sender as its actor", event.topic);
        assert_no_trace_of(&sender, SENDER, &event.data, &format!("the {} event", event.topic));
    }

    match next_push(&mut pushes).await.expect("the owner is pushed the question") {
        Push::QuestionReceived(received) => {
            assert_no_trace_of(&sender, SENDER, &*received, "the realtime push");
            assert_eq!(received.question.as_option().unwrap().id, sent.id);
            assert_eq!(received.counts.as_option().unwrap().unread, 1);
        }
        other => panic!("unexpected push {other:?}"),
    }

    // Every RPC the owner can call about the question, as it goes over the wire.
    let id = sent.id.clone();
    let seen = [
        ("ListInbox", ctx.json(BOX, "ListInbox", &owner, &ListInboxRequest::default()).await),
        ("GetInboxCounts", ctx.json(BOX, "GetInboxCounts", &owner, &GetInboxCountsRequest::default()).await),
        ("GetQuestion", ctx.json(BOX, "GetQuestion", &owner, &question_id(&id)).await),
        (
            "MarkQuestionsRead",
            ctx.json(BOX, "MarkQuestionsRead", &owner, &MarkQuestionsReadRequest { all: true, ..Default::default() })
                .await,
        ),
        (
            "MoveQuestion",
            ctx.json(
                BOX,
                "MoveQuestion",
                &owner,
                &MoveQuestionRequest {
                    question_id: id.clone(),
                    folder: QuestionFolder::QUESTION_FOLDER_ARCHIVED.into(),
                    ..Default::default()
                },
            )
            .await,
        ),
        (
            "BlockSender",
            ctx.json(
                BOX,
                "BlockSender",
                &owner,
                &BlockSenderRequest {
                    target: Some(block_sender_request::Target::QuestionId(id.clone())),
                    ..Default::default()
                },
            )
            .await,
        ),
        ("ListSenderBlocks", ctx.json(BOX, "ListSenderBlocks", &owner, &ListSenderBlocksRequest::default()).await),
        (
            "AnswerQuestion",
            ctx.json(
                BOX,
                "AnswerQuestion",
                &owner,
                &AnswerQuestionRequest { question_id: id.clone(), text: "ラーメン!".into(), ..Default::default() },
            )
            .await,
        ),
        (
            "UpdateAnswer",
            ctx.json(
                BOX,
                "UpdateAnswer",
                &owner,
                &UpdateAnswerRequest {
                    question_id: id.clone(),
                    text: Some("やっぱりカレー!".into()),
                    ..Default::default()
                },
            )
            .await,
        ),
        (
            "ListInbox(ANSWERED)",
            ctx.json(
                BOX,
                "ListInbox",
                &owner,
                &ListInboxRequest { folder: QuestionFolder::QUESTION_FOLDER_ANSWERED.into(), ..Default::default() },
            )
            .await,
        ),
        (
            "ListAnswers",
            ctx.json(
                BOX,
                "ListAnswers",
                &owner,
                &ListAnswersRequest { user: owner.id.to_string(), ..Default::default() },
            )
            .await,
        ),
        (
            "GetBox",
            ctx.json(BOX, "GetBox", &owner, &GetBoxRequest { user: owner.id.to_string(), ..Default::default() }).await,
        ),
        ("GetSettings", ctx.json(BOX, "GetSettings", &owner, &GetSettingsRequest::default()).await),
    ];
    for (rpc, response) in &seen {
        assert_no_trace_of(&sender, SENDER, response, rpc);
    }
    // The question itself is there, so the checks above looked at something.
    assert_eq!(seen[0].1["questions"][0]["id"], serde_json::json!(id));
    assert_eq!(seen[0].1["questions"][0]["anonymous"], serde_json::json!(true));

    // A reader of the published answer learns as little.
    for (rpc, response) in [
        (
            "ListAnswers",
            ctx.json(
                BOX,
                "ListAnswers",
                &reader,
                &ListAnswersRequest { user: owner.handle.clone(), ..Default::default() },
            )
            .await,
        ),
        ("GetQuestion", ctx.json(BOX, "GetQuestion", &reader, &question_id(&id)).await),
    ] {
        assert_no_trace_of(&sender, SENDER, &response, &format!("{rpc} for a reader"));
    }

    // Answering wrote events and told the asker; none of it reaches the
    // owner with the asker in it.
    for event in ctx.deliver().await {
        if event.data["user_id"] != serde_json::json!(sender.id) {
            assert_no_trace_of(&sender, SENDER, &event.data, &format!("the {} event", event.topic));
        }
        assert_ne!(event.actor_id, Some(sender.id));
    }
    for notice in ctx.all_notifications_for(owner.id).await {
        assert!(
            notice.actor_id.is_none() && notice.data["actor_id"].is_null(),
            "a notification about an anonymous question has no actor"
        );
        assert_no_trace_of(&sender, SENDER, &notice.data, "a notification to the owner");
    }
    assert_eq!(ctx.notifications_for(owner.id, "ask.question").await.len(), 1);
}

#[tokio::test]
async fn a_signed_question_names_its_asker_because_the_asker_chose_to() {
    let ctx = start().await;
    let (owner, sender) = (ctx.user("Owner").await, ctx.user(SENDER).await);
    ctx.open_box(&owner, true).await;
    let sent = ctx.try_ask(&sender, &owner, "今度あそぼ!", true).await.unwrap();
    ctx.deliver().await;
    assert!(!sent.anonymous);

    let inbox = ctx.inbox(&owner, QuestionFolder::QUESTION_FOLDER_INBOX).await;
    assert_eq!(inbox.len(), 1);
    assert!(!inbox[0].anonymous);
    assert_eq!(inbox[0].asker.as_option().expect("the asker is named").id, sender.id.to_string());
    let notices = ctx.notifications_for(owner.id, "ask.question").await;
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].actor_id, Some(sender.id));

    // A box can refuse signatures.
    ctx.update_settings(&owner, UpdateSettingsRequest { allow_signed: Some(false), ..Default::default() }).await;
    assert_eq!(ctx.try_ask(&sender, &owner, "もう一回!", true).await.unwrap_err().reason, "SIGNED_NOT_ALLOWED");
}

#[tokio::test]
async fn a_blocked_sender_is_told_exactly_what_anyone_is_told() {
    let ctx = start().await;
    let (owner, blocked, other) = (ctx.user("Owner").await, ctx.user(SENDER).await, ctx.user("Other").await);
    ctx.open_box(&owner, true).await;
    let first = ctx.ask(&blocked, &owner, "一つ目").await;
    ctx.ask(&other, &owner, "一つ目").await;
    ctx.block_sender_of_question(&owner, &first.id).await;
    let mut pushes = ctx.app.listen_user(owner.id).await;
    ctx.new_events().await;

    // The same request from both: one is dropped, one arrives.
    let request =
        |text: &str| AskQuestionRequest { user: owner.id.to_string(), text: text.to_owned(), ..Default::default() };
    let dropped = ctx.json(BOX, "AskQuestion", &blocked, &request("二つ目")).await;
    let dropped_events = ctx.new_events().await;
    let arrived = ctx.json(BOX, "AskQuestion", &other, &request("二つ目")).await;
    let arrived_events = ctx.new_events().await;
    assert_eq!(
        told(dropped["question"].clone()),
        told(arrived["question"].clone()),
        "the response does not depend on the block"
    );
    assert_eq!(dropped["question"]["sentState"], serde_json::json!("SENT_STATE_WAITING"));
    assert_eq!(shape(&dropped_events), shape(&arrived_events), "the request writes the same events either way");
    assert_eq!(dropped_events.len(), 1);
    assert_eq!((dropped_events[0].topic.as_str(), dropped_events[0].actor_id), ("ask.question.stored", None));
    ctx.handle(&dropped_events).await;
    ctx.handle(&arrived_events).await;

    // What each of them can look at afterwards.
    let dropped_id = dropped["question"]["id"].as_str().unwrap().to_owned();
    let arrived_id = arrived["question"]["id"].as_str().unwrap().to_owned();
    let listed = |questions: Vec<Question>| {
        questions
            .into_iter()
            .map(|q| (q.text, q.sent_state.as_known(), q.sent_by_me, q.answer.is_set()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        listed(ctx.asked(&blocked).await),
        listed(ctx.asked(&other).await),
        "both lists show two waiting questions"
    );
    assert_eq!(ctx.asked(&blocked).await.len(), 2);
    let by_id_dropped = ctx.json(BOX, "GetQuestion", &blocked, &question_id(&dropped_id)).await;
    let by_id_arrived = ctx.json(BOX, "GetQuestion", &other, &question_id(&arrived_id)).await;
    assert_eq!(told(by_id_dropped["question"].clone()), told(by_id_arrived["question"].clone()));

    // Only one of the two reached the owner, who was told about one.
    let inbox = ctx.inbox(&owner, QuestionFolder::QUESTION_FOLDER_INBOX).await;
    let ids: Vec<&str> = inbox.iter().map(|q| q.id.as_str()).collect();
    assert!(ids.contains(&arrived_id.as_str()) && !ids.contains(&dropped_id.as_str()));
    assert_eq!(ctx.counts(&owner).await.inbox, 3, "the two first questions and the one that arrived");
    match next_push(&mut pushes).await.expect("the question that arrived is pushed") {
        Push::QuestionReceived(received) => assert_eq!(received.question.as_option().unwrap().id, arrived_id),
        other => panic!("unexpected push {other:?}"),
    }
    assert!(stays_quiet(&mut pushes).await, "nothing is pushed for the dropped question");
    let owner_view =
        ctx.call::<_, GetQuestionResponse>(BOX, "GetQuestion", &owner, &question_id(&dropped_id)).await.unwrap_err();
    assert_eq!(owner_view.reason, "QUESTION_NOT_FOUND", "the owner cannot open a dropped question even by ID");

    // Taking a question back works the same for both.
    for (who, id) in [(&blocked, &dropped_id), (&other, &arrived_id)] {
        let _: WithdrawQuestionResponse = ctx
            .call(
                BOX,
                "WithdrawQuestion",
                who,
                &WithdrawQuestionRequest { question_id: id.clone(), ..Default::default() },
            )
            .await
            .unwrap();
    }
    assert_eq!(listed(ctx.asked(&blocked).await), listed(ctx.asked(&other).await));
    assert_eq!(ctx.asked(&blocked).await.len(), 1);
}

#[tokio::test]
async fn limits_count_a_blocked_senders_questions_like_everyone_elses() {
    let cfg = tokuly_ask::AskConfig { pair_sends_per_day: 3, box_receives_per_hour: 7, ..Default::default() };
    let ctx = start_with(cfg).await;
    let (owner, blocked, other, third) =
        (ctx.user("Owner").await, ctx.user(SENDER).await, ctx.user("Other").await, ctx.user("Third").await);
    ctx.open_box(&owner, true).await;
    let first = ctx.ask(&blocked, &owner, "1").await;
    ctx.block_sender_of_question(&owner, &first.id).await;
    ctx.ask(&other, &owner, "1").await;

    // Per pair: both hit the limit at their fourth question.
    for sender in [&blocked, &other] {
        for n in 2..=3 {
            ctx.try_ask(sender, &owner, &n.to_string(), false)
                .await
                .unwrap_or_else(|e| panic!("question {n} of {}: {e}", sender.handle));
        }
        let over = ctx.try_ask(sender, &owner, "4", false).await.unwrap_err();
        assert_eq!(
            (over.code.as_str(), over.reason.as_str()),
            ("resource_exhausted", "RATE_LIMITED"),
            "{}",
            sender.handle
        );
    }
    // Per box: the dropped questions took their place in the owner's hour,
    // so a second account cannot measure whether they "arrived". Six
    // questions were accepted so far (three of them dropped); the limit of
    // seven leaves room for one more.
    ctx.try_ask(&third, &owner, "7", false).await.unwrap();
    let busy = ctx.try_ask(&third, &owner, "8", false).await.unwrap_err();
    assert_eq!((busy.code.as_str(), busy.reason.as_str()), ("resource_exhausted", "BOX_BUSY"));
    ctx.deliver().await;
    assert_eq!(
        ctx.counts(&owner).await.inbox,
        5,
        "one from the blocked sender before the block, three from another, one from a third"
    );
}

#[tokio::test]
async fn blocking_a_sender_tells_the_owner_nothing() {
    let ctx = start().await;
    let (owner, sender, other) = (ctx.user("Owner").await, ctx.user(SENDER).await, ctx.user("Other").await);
    ctx.open_box(&owner, true).await;
    let first = ctx.ask(&sender, &owner, "一つ目").await;
    let second = ctx.ask(&sender, &owner, "二つ目").await;
    ctx.ask(&other, &owner, "ほかの人から").await;

    // Blocking removes nothing that is already there: if the sender's other
    // question vanished, the owner would know the two came from one person.
    let block = ctx.block_sender_of_question(&owner, &first.id).await;
    assert_eq!((block.source.as_known(), block.preview.as_str()), (Some(BlockSource::BLOCK_SOURCE_QUESTION), "一つ目"));
    assert_eq!(ctx.inbox(&owner, QuestionFolder::QUESTION_FOLDER_INBOX).await.len(), 3);
    assert_eq!(ctx.counts(&owner).await.inbox, 3);

    // Two blocked items are two blocks, also when one person sent both.
    let again = ctx.block_sender_of_question(&owner, &first.id).await;
    assert_eq!(again.id, block.id, "blocking the same item twice is one block");
    let other_block = ctx.block_sender_of_question(&owner, &second.id).await;
    assert_ne!(other_block.id, block.id);
    let blocks: ListSenderBlocksResponse =
        ctx.call(BOX, "ListSenderBlocks", &owner, &ListSenderBlocksRequest::default()).await.unwrap();
    assert_eq!(blocks.blocks.len(), 2);
    assert_no_trace_of(&sender, SENDER, &blocks, "the list of blocks");
    for block in &blocks.blocks {
        assert_ne!(block.id, sender.id.to_string());
    }

    // The sender is blocked while either block stands.
    let lift = |id: String| UnblockSenderRequest { block_id: id, ..Default::default() };
    let _: UnblockSenderResponse = ctx.call(BOX, "UnblockSender", &owner, &lift(block.id.clone())).await.unwrap();
    ctx.ask(&sender, &owner, "まだブロック中").await;
    assert_eq!(ctx.counts(&owner).await.inbox, 3);
    let _: UnblockSenderResponse = ctx.call(BOX, "UnblockSender", &owner, &lift(other_block.id.clone())).await.unwrap();
    ctx.ask(&sender, &owner, "もう届く").await;
    assert_eq!(ctx.counts(&owner).await.inbox, 4);
    let gone =
        ctx.call::<_, UnblockSenderResponse>(BOX, "UnblockSender", &owner, &lift(block.id.clone())).await.unwrap_err();
    assert_eq!(gone.reason, "BLOCK_NOT_FOUND");

    // Only what was sent to the caller can be blocked.
    let request = BlockSenderRequest {
        target: Some(block_sender_request::Target::QuestionId(first.id.clone())),
        ..Default::default()
    };
    assert_eq!(
        ctx.call::<_, BlockSenderResponse>(BOX, "BlockSender", &other, &request).await.unwrap_err().reason,
        "BLOCK_TARGET_NOT_FOUND"
    );
    assert_eq!(
        ctx.call::<_, BlockSenderResponse>(BOX, "BlockSender", &owner, &BlockSenderRequest::default())
            .await
            .unwrap_err()
            .reason,
        "BLOCK_TARGET_REQUIRED"
    );
}

#[tokio::test]
async fn a_block_outlives_the_account_it_was_made_of() {
    let ctx = start().await;
    let (owner, sender) = (ctx.user("Owner").await, ctx.user(SENDER).await);
    ctx.open_box(&owner, true).await;
    let question = ctx.ask(&sender, &owner, "だれでしょう").await;
    ctx.block_sender_of_question(&owner, &question.id).await;

    ctx.announce("identity.user.deleted", serde_json::json!({ "user_id": sender.id })).await;

    // A block that vanished would tell the owner that the sender left.
    let blocks: ListSenderBlocksResponse =
        ctx.call(BOX, "ListSenderBlocks", &owner, &ListSenderBlocksRequest::default()).await.unwrap();
    assert_eq!(blocks.blocks.len(), 1);
    // The question stays in the box, anonymous, with nobody behind it.
    let inbox = ctx.inbox(&owner, QuestionFolder::QUESTION_FOLDER_INBOX).await;
    assert_eq!(inbox.len(), 1);
    assert!(inbox[0].anonymous && inbox[0].asker.is_unset());
    let sender_left: Option<Uuid> = sqlx::query_scalar("SELECT sender_id FROM ask_questions WHERE id = $1")
        .bind(Uuid::parse_str(&question.id).unwrap())
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(sender_left, None);
}

#[tokio::test]
async fn moderators_see_who_sent_a_question_and_nobody_else_does() {
    let ctx = start().await;
    let (owner, sender, stranger) = (ctx.user("Owner").await, ctx.user(SENDER).await, ctx.user("Stranger").await);
    let moderator = ctx.app.user_with_roles("Moderator", &["moderator"]).await;
    ctx.open_box(&owner, true).await;
    let question = ctx.ask(&sender, &owner, "いやなことを書いた質問").await;
    let request = InspectQuestionRequest { question_id: question.id.clone(), ..Default::default() };

    let seen: InspectQuestionResponse = ctx.call(MODERATION, "InspectQuestion", &moderator, &request).await.unwrap();
    let record = seen.record.as_option().unwrap();
    assert_eq!(record.sender.as_option().expect("moderators see the sender").id, sender.id.to_string());
    assert_eq!(record.recipient.as_option().unwrap().id, owner.id.to_string());
    assert!(!record.signed && !record.dropped && !record.sender_banned);
    assert_eq!(seen.text, "いやなことを書いた質問");

    // Everyone else is refused, the owner included.
    for user in [&owner, &sender, &stranger] {
        let refused =
            ctx.call::<_, InspectQuestionResponse>(MODERATION, "InspectQuestion", user, &request).await.unwrap_err();
        assert_eq!((refused.code.as_str(), refused.reason.as_str()), ("permission_denied", "FORBIDDEN"));
    }
    let guest = ctx
        .app
        .call::<_, InspectQuestionResponse>(&format!("{MODERATION}/InspectQuestion"), None, &request)
        .await
        .unwrap_err();
    assert_eq!(guest.code, "unauthenticated");

    // Looking leaves a trace; being refused does not.
    let traces: Vec<(Option<Uuid>, String)> =
        sqlx::query_as("SELECT staff_id, kind FROM ask_inspections WHERE target_id = $1")
            .bind(Uuid::parse_str(&question.id).unwrap())
            .fetch_all(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!(traces, vec![(Some(moderator.id), "question".to_owned())]);

    // A question its recipient deleted can still be looked up.
    let _: DeleteQuestionResponse = ctx
        .call(
            BOX,
            "DeleteQuestion",
            &owner,
            &DeleteQuestionRequest { question_id: question.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let deleted: InspectQuestionResponse = ctx.call(MODERATION, "InspectQuestion", &moderator, &request).await.unwrap();
    assert!(deleted.removed_at.is_set());
    assert_eq!(deleted.record.as_option().unwrap().sender.as_option().unwrap().id, sender.id.to_string());
    let missing = InspectQuestionRequest { question_id: Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(
        ctx.call::<_, InspectQuestionResponse>(MODERATION, "InspectQuestion", &moderator, &missing)
            .await
            .unwrap_err()
            .reason,
        "QUESTION_NOT_FOUND"
    );
}

#[tokio::test]
async fn the_subject_resolver_names_the_sender_to_staff_only_and_leaves_a_trace() {
    let ctx = start().await;
    let (owner, sender, stranger) = (ctx.user("Owner").await, ctx.user(SENDER).await, ctx.user("Stranger").await);
    let moderator = ctx.app.user_with_roles("Moderator", &["moderator"]).await;
    ctx.open_box(&owner, true).await;
    let question = ctx.ask(&sender, &owner, "通報される質問").await;
    let subjects = &ctx.app.core.subjects;
    let question_type = SubjectType::SUBJECT_TYPE_QUESTION;

    // Its recipient can report it, and is told it belongs to nobody.
    let for_owner = subjects.resolve(question_type, &question.id, Some(owner.id)).await.unwrap();
    assert!(for_owner.owner_id.is_nil());
    assert!(!for_owner.interactions_enabled);
    assert_eq!(for_owner.title, "通報される質問");

    // Staff are told who wrote it, once on the record.
    for _ in 0..2 {
        let for_staff = subjects.resolve(question_type, &question.id, Some(moderator.id)).await.unwrap();
        assert_eq!(for_staff.owner_id, sender.id);
    }
    let traces: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ask_inspections WHERE target_id = $1 AND staff_id = $2 AND kind = 'question'",
    )
    .bind(Uuid::parse_str(&question.id).unwrap())
    .bind(moderator.id)
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(traces, 1, "repeated lookups within the hour are one trace");

    // Nobody else finds it: not a stranger, not a guest, not its sender.
    for viewer in [Some(stranger.id), None, Some(sender.id)] {
        assert_eq!(
            subjects.resolve(question_type, &question.id, viewer).await.unwrap_err().reason,
            "SUBJECT_NOT_FOUND",
            "{viewer:?}"
        );
    }

    // Once answered in public it is the owner's post, for everyone.
    ctx.answer(&owner, &question.id, "こたえ", AnswerAudience::ANSWER_AUDIENCE_PUBLIC).await;
    for viewer in [Some(owner.id), Some(stranger.id), None, Some(sender.id), Some(moderator.id)] {
        let published = subjects.resolve(question_type, &question.id, viewer).await.unwrap();
        assert_eq!(published.owner_id, owner.id, "{viewer:?}");
        assert!(published.interactions_enabled);
    }

    // Likes and comments are counted on the question.
    subjects.counter_changed(question_type, Uuid::parse_str(&question.id).unwrap(), "like_count", 2).await.unwrap();
    subjects.counter_changed(question_type, Uuid::parse_str(&question.id).unwrap(), "comment_count", 1).await.unwrap();
    let page: GetQuestionResponse = ctx.call(BOX, "GetQuestion", &stranger, &question_id(&question.id)).await.unwrap();
    let shown = page.question.as_option().unwrap();
    assert_eq!((shown.like_count, shown.comment_count), (2, 1));
}

#[tokio::test]
async fn a_moderator_is_an_ordinary_owner_of_their_own_box() {
    let ctx = start().await;
    let sender = ctx.user(SENDER).await;
    let moderator = ctx.app.user_with_roles("Moderator", &["moderator"]).await;
    ctx.open_box(&moderator, true).await;
    let question = ctx.ask(&sender, &moderator, "モデレーターへの質問").await;

    // Through the generic domains a moderator sees a question sent to
    // themselves like any recipient does: owned by nobody, nothing recorded.
    let own = ctx
        .app
        .core
        .subjects
        .resolve(SubjectType::SUBJECT_TYPE_QUESTION, &question.id, Some(moderator.id))
        .await
        .unwrap();
    assert!(own.owner_id.is_nil());
    let traces: i64 = sqlx::query_scalar("SELECT count(*) FROM ask_inspections WHERE target_id = $1")
        .bind(Uuid::parse_str(&question.id).unwrap())
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(traces, 0);
    assert_no_trace_of(
        &sender,
        SENDER,
        &ctx.json(BOX, "ListInbox", &moderator, &ListInboxRequest::default()).await,
        "a moderator's own inbox",
    );
}

#[tokio::test]
async fn an_image_needs_a_signature() {
    let ctx = start().await;
    let (owner, sender) = (ctx.user("Owner").await, ctx.user(SENDER).await);
    ctx.open_box(&owner, true).await;
    let image = ready_image(&ctx.app.core, sender.id, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await.unwrap();
    let with_image = |signed: bool| AskQuestionRequest {
        user: owner.id.to_string(),
        text: "これ見て".into(),
        signed,
        image_asset_id: image.id.to_string(),
        ..Default::default()
    };
    let ask = |signed: bool| {
        let request = with_image(signed);
        let (ctx, sender) = (&ctx, &sender);
        async move { ctx.call::<_, AskQuestionResponse>(BOX, "AskQuestion", sender, &request).await }
    };

    // An upload can be traced to its uploader, so it would give an
    // anonymous sender away: refused, whatever the box allows.
    assert_eq!(ask(false).await.unwrap_err().reason, "IMAGE_REQUIRES_SIGNATURE");
    assert_eq!(
        ask(true).await.unwrap_err().reason,
        "IMAGES_NOT_ALLOWED",
        "images are off until the owner turns them on"
    );
    ctx.update_settings(&owner, UpdateSettingsRequest { allow_images: Some(true), ..Default::default() }).await;
    assert_eq!(ask(false).await.unwrap_err().reason, "IMAGE_REQUIRES_SIGNATURE");

    let sent = ask(true).await.unwrap();
    assert_eq!(sent.question.as_option().unwrap().image.as_option().expect("the image").asset_id, image.id.to_string());
    ctx.deliver().await;
    let inbox = ctx.inbox(&owner, QuestionFolder::QUESTION_FOLDER_INBOX).await;
    assert_eq!(inbox[0].image.as_option().expect("the owner sees the image").asset_id, image.id.to_string());
    assert!(
        tokuly_media::assets::is_attached(&ctx.app.core, image.id).await.unwrap(),
        "the image is held by the question"
    );

    // Someone else's upload cannot be attached.
    let other = ctx.user("Other").await;
    let request = with_image(true);
    assert_eq!(
        ctx.call::<_, AskQuestionResponse>(BOX, "AskQuestion", &other, &request).await.unwrap_err().reason,
        "ASSET_NOT_FOUND"
    );
}

#[tokio::test]
async fn a_banned_senders_questions_disappear_while_the_ban_lasts() {
    let ctx = start().await;
    let (owner, sender, other) = (ctx.user("Owner").await, ctx.user(SENDER).await, ctx.user("Other").await);
    ctx.open_box(&owner, true).await;
    let from_banned = ctx.ask(&sender, &owner, "あとで消える").await;
    ctx.ask(&other, &owner, "のこる").await;
    let answered = ctx.ask(&sender, &owner, "答えたやつ").await;
    ctx.answer(&owner, &answered.id, "こたえ", AnswerAudience::ANSWER_AUDIENCE_PUBLIC).await;
    assert_eq!((ctx.counts(&owner).await.inbox, ctx.counts(&owner).await.answered), (2, 1));

    ctx.app.identity.set_banned(sender.id, Some("harassment")).await.unwrap();
    let events = ctx.deliver().await;
    let texts = |questions: Vec<Question>| questions.into_iter().map(|q| q.text).collect::<Vec<_>>();
    assert_eq!(texts(ctx.inbox(&owner, QuestionFolder::QUESTION_FOLDER_INBOX).await), vec!["のこる"]);
    assert_eq!(
        (ctx.counts(&owner).await.inbox, ctx.counts(&owner).await.unread, ctx.counts(&owner).await.answered),
        (1, 1, 0)
    );
    let hidden = ctx
        .call::<_, GetQuestionResponse>(BOX, "GetQuestion", &owner, &question_id(&from_banned.id))
        .await
        .unwrap_err();
    assert_eq!(hidden.reason, "QUESTION_NOT_FOUND");
    let page: ListAnswersResponse = ctx
        .call(BOX, "ListAnswers", &other, &ListAnswersRequest { user: owner.id.to_string(), ..Default::default() })
        .await
        .unwrap();
    assert!(page.questions.is_empty(), "an answer to a banned account's question leaves the page");
    assert!(
        events.iter().any(|e| e.topic == "search.changed"
            && e.data["id"] == serde_json::json!(answered.id)
            && e.data["doc"].is_null()),
        "and the search index"
    );

    // Lifting the ban brings them back.
    ctx.app.identity.set_banned(sender.id, None).await.unwrap();
    let events = ctx.deliver().await;
    assert_eq!(ctx.counts(&owner).await.inbox, 2);
    assert_eq!(ctx.counts(&owner).await.answered, 1);
    assert!(events.iter().any(|e| e.topic == "search.changed"
        && e.data["id"] == serde_json::json!(answered.id)
        && !e.data["doc"].is_null()));
}

#[tokio::test]
async fn an_old_unanswered_question_leaves_the_senders_list_whatever_became_of_it() {
    let ctx = start().await;
    let (owner, blocked, other): (TestUser, TestUser, TestUser) =
        (ctx.user("Owner").await, ctx.user(SENDER).await, ctx.user("Other").await);
    ctx.open_box(&owner, true).await;
    let first = ctx.ask(&blocked, &owner, "ブロックのきっかけ").await;
    ctx.block_sender_of_question(&owner, &first.id).await;
    let dropped = ctx.ask(&blocked, &owner, "届かない").await;
    let ignored = ctx.ask(&other, &owner, "届くけど放置").await;
    let answered = ctx.ask(&other, &owner, "答えてもらえる").await;
    ctx.answer(&owner, &answered.id, "こたえ", AnswerAudience::ANSWER_AUDIENCE_PUBLIC).await;

    // Half a year later (the retention): the erasure of the dropped
    // question must look to its sender like what happens to any old
    // unanswered question.
    sqlx::query("UPDATE ask_questions SET created_at = created_at - interval '181 days' WHERE owner_id = $1")
        .bind(owner.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert!(ctx.asked(&blocked).await.is_empty());
    let left: Vec<String> = ctx.asked(&other).await.into_iter().map(|q| q.text).collect();
    assert_eq!(left, vec!["答えてもらえる"], "answered questions stay; the ignored one aged out like the dropped one");
    for (who, id) in [(&blocked, &dropped.id), (&other, &ignored.id)] {
        let gone = ctx.call::<_, GetQuestionResponse>(BOX, "GetQuestion", who, &question_id(id)).await.unwrap_err();
        assert_eq!(gone.reason, "QUESTION_NOT_FOUND", "by ID neither is found, though one of them is still in the box");
    }
    assert_eq!(ctx.counts(&owner).await.inbox, 2, "the owner still has what reached them");

    // The purge erases what was only kept for moderators.
    let erased: i64 = sqlx::query_scalar("SELECT count(*) FROM ask_questions WHERE id = $1")
        .bind(Uuid::parse_str(&dropped.id).unwrap())
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(erased, 1);
    ctx.ask.purge().await.unwrap();
    let erased: i64 = sqlx::query_scalar("SELECT count(*) FROM ask_questions WHERE id = $1")
        .bind(Uuid::parse_str(&dropped.id).unwrap())
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(erased, 0);
    assert_eq!(ctx.counts(&owner).await.inbox, 2);
}
