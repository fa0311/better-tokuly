//! What the domain does when something elsewhere changes (accounts,
//! takedowns, uploads, friendships), and what it plugs into: profiles and
//! the personal data export.
mod common;

use common::*;
use tokuly_media::testing::ready_image;
use tokuly_proto::{
    ask::*,
    common::SubjectType,
    identity::{GetProfileRequest, GetProfileResponse, get_profile_request::By},
    media::AssetPurpose,
};
use tokuly_testkit::TestUser;
use uuid::Uuid;

/// A name no response can contain by accident.
const SENDER: &str = "Zebedee Quillfeather";

async fn count(ctx: &Ctx, sql: &'static str, id: Uuid) -> i64 {
    sqlx::query_scalar(sql).bind(id).fetch_one(&ctx.app.core.db).await.expect("count")
}

async fn post_prompt(ctx: &Ctx, owner: &TestUser, text: &str) -> Prompt {
    let response: CreatePromptResponse = ctx
        .call(PROMPTS, "CreatePrompt", owner, &CreatePromptRequest { text: text.to_owned(), ..Default::default() })
        .await
        .unwrap();
    ctx.deliver().await;
    response.prompt.as_option().cloned().unwrap()
}

async fn respond(ctx: &Ctx, sender: &TestUser, prompt: &Prompt, text: &str, signed: bool) -> PromptResponse {
    let request =
        RespondToPromptRequest { prompt_id: prompt.id.clone(), text: text.to_owned(), signed, ..Default::default() };
    let response: RespondToPromptResponse = ctx.call(PROMPTS, "RespondToPrompt", sender, &request).await.unwrap();
    ctx.deliver().await;
    response.response.as_option().cloned().unwrap()
}

async fn cards(ctx: &Ctx, viewer: Option<&TestUser>, of: &TestUser) -> Vec<tokuly_proto::identity::ProfileCard> {
    let request = GetProfileRequest { by: Some(By::UserId(of.id.to_string())), ..Default::default() };
    let response: GetProfileResponse =
        ctx.app.call("tokuly.identity.v1.ProfileService/GetProfile", viewer, &request).await.expect("profile");
    response.profile.as_option().expect("profile").cards.clone()
}

#[tokio::test]
async fn a_deleted_account_takes_its_box_along_and_leaves_what_it_sent_anonymous() {
    let ctx = start().await;
    let (leaver, stayer) = (ctx.user(SENDER).await, ctx.user("Stayer").await);
    ctx.befriend(&leaver, &stayer).await;
    ctx.crowd(&leaver).await;
    ctx.open_box(&leaver, true).await;
    ctx.open_box(&stayer, true).await;

    // The leaver's box, with a public answer.
    let received = ctx.ask(&stayer, &leaver, "やめる人への質問").await;
    ctx.answer(&leaver, &received.id, "こたえ", AnswerAudience::ANSWER_AUDIENCE_PUBLIC).await;
    // What the leaver sent: a signed and an anonymous question, a signed response.
    let signed = ctx.try_ask(&leaver, &stayer, "名前つきで聞いた", true).await.unwrap();
    let anonymous = ctx.ask(&leaver, &stayer, "匿名で聞いた").await;
    let stayers_prompt = post_prompt(&ctx, &stayer, "のこる人のお題").await;
    let response = respond(&ctx, &leaver, &stayers_prompt, "名前つきの回答", true).await;
    let leavers_prompt = post_prompt(&ctx, &leaver, "やめる人のお題").await;
    respond(&ctx, &stayer, &leavers_prompt, "のこる人の回答", false).await;
    ctx.block_sender_of_question(&leaver, &received.id).await;
    // Flames in both directions (the bank is loaded by the first poll).
    let _ = ctx.call::<_, GetRoundResponse>(POLLS, "GetRound", &leaver, &GetRoundRequest::default()).await;
    for (receiver, sender) in [(leaver.id, stayer.id), (stayer.id, leaver.id)] {
        sqlx::query("INSERT INTO ask_flames (receiver_id, sender_id, question_id, delivered_at) SELECT $1, $2, id, now() FROM ask_poll_questions ORDER BY id LIMIT 1")
            .bind(receiver)
            .bind(sender)
            .execute(&ctx.app.core.db)
            .await
            .unwrap();
    }
    ctx.new_events().await;

    for _ in 0..2 {
        // Twice: the event can arrive more than once.
        ctx.announce("identity.user.deleted", serde_json::json!({ "user_id": leaver.id })).await;
    }

    // Their box, answers, prompts, blocks, settings and received flames are gone.
    for (what, sql) in [
        ("questions in their box", "SELECT count(*) FROM ask_questions WHERE owner_id = $1"),
        ("their prompts", "SELECT count(*) FROM ask_prompts WHERE owner_id = $1"),
        ("responses to their prompts", "SELECT count(*) FROM ask_prompt_responses WHERE owner_id = $1"),
        ("their blocks", "SELECT count(*) FROM ask_sender_blocks WHERE owner_id = $1"),
        ("their settings", "SELECT count(*) FROM ask_settings WHERE user_id = $1"),
        ("flames they received", "SELECT count(*) FROM ask_flames WHERE receiver_id = $1"),
        (
            "anything that still names them as a sender",
            "SELECT (SELECT count(*) FROM ask_questions WHERE sender_id = $1) + (SELECT count(*) FROM ask_prompt_responses WHERE sender_id = $1) + (SELECT count(*) FROM ask_flames WHERE sender_id = $1)",
        ),
    ] {
        assert_eq!(count(&ctx, sql, leaver.id).await, 0, "{what}");
    }
    let unindexed = ctx.new_events().await;
    assert!(unindexed.iter().any(|e| e.topic == "search.changed"
        && e.data["id"] == serde_json::json!(received.id)
        && e.data["doc"].is_null()));

    // What they sent stays with its recipient, without the name.
    let inbox = ctx.inbox(&stayer, QuestionFolder::QUESTION_FOLDER_INBOX).await;
    assert_eq!(inbox.len(), 2);
    assert!(inbox.iter().all(|q| q.anonymous && q.asker.is_unset()), "the signature went with the account");
    assert!(inbox.iter().any(|q| q.id == signed.id) && inbox.iter().any(|q| q.id == anonymous.id));
    assert_no_trace_of(&leaver, SENDER, &inbox, "the inbox after the deletion");
    let listing = ListPromptResponsesRequest { prompt_id: stayers_prompt.id.clone(), ..Default::default() };
    let responses: ListPromptResponsesResponse =
        ctx.call(PROMPTS, "ListPromptResponses", &stayer, &listing).await.unwrap();
    assert_eq!(responses.responses.len(), 1);
    assert!(
        responses.responses[0].id == response.id
            && responses.responses[0].anonymous
            && responses.responses[0].responder.is_unset()
    );
    assert_eq!(ctx.flames(&stayer).await.len(), 1, "a compliment stays with whoever got it");
}

#[tokio::test]
async fn a_takedown_removes_a_question_for_everyone() {
    let ctx = start().await;
    let (owner, asker, reader) = (ctx.user("Owner").await, ctx.user("Asker").await, ctx.user("Reader").await);
    let moderator = ctx.app.user_with_roles("Moderator", &["moderator"]).await;
    ctx.open_box(&owner, true).await;
    let taken = ctx.ask(&asker, &owner, "消される質問").await;
    let kept = ctx.ask(&asker, &owner, "のこる質問").await;
    for question in [&taken, &kept] {
        ctx.answer(&owner, &question.id, "こたえ", AnswerAudience::ANSWER_AUDIENCE_PUBLIC).await;
    }
    let removed = |subject_type: i32, id: &str| serde_json::json!({ "subject_type": subject_type, "subject_id": id, "reason": "harassment", "moderator_id": moderator.id });
    let question_type = SubjectType::SUBJECT_TYPE_QUESTION as i32;
    ctx.new_events().await;

    // Another kind of subject with the same ID is none of our business; an
    // unknown ID is done already.
    ctx.announce("admin.content.removed", removed(SubjectType::SUBJECT_TYPE_NOTE as i32, &kept.id)).await;
    ctx.announce("admin.content.removed", removed(question_type, &Uuid::now_v7().to_string())).await;
    for _ in 0..2 {
        ctx.announce("admin.content.removed", removed(question_type, &taken.id)).await;
    }

    let page: ListAnswersResponse = ctx
        .call(BOX, "ListAnswers", &reader, &ListAnswersRequest { user: owner.id.to_string(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(page.questions.iter().map(|q| q.id.as_str()).collect::<Vec<_>>(), vec![kept.id.as_str()]);
    assert_eq!(ctx.counts(&owner).await.answered, 1);
    assert_eq!(
        ctx.asked(&asker).await.iter().map(|q| q.id.as_str()).collect::<Vec<_>>(),
        vec![kept.id.as_str()],
        "gone for its sender too"
    );
    for viewer in [&owner, &asker, &reader, &moderator] {
        let request = GetQuestionRequest { question_id: taken.id.clone(), ..Default::default() };
        assert_eq!(
            ctx.call::<_, GetQuestionResponse>(BOX, "GetQuestion", viewer, &request).await.unwrap_err().reason,
            "QUESTION_NOT_FOUND"
        );
        let resolved =
            ctx.app.core.subjects.resolve(SubjectType::SUBJECT_TYPE_QUESTION, &taken.id, Some(viewer.id)).await;
        assert_eq!(resolved.unwrap_err().reason, "SUBJECT_NOT_FOUND");
    }
    let events = ctx.new_events().await;
    assert_eq!(
        events
            .iter()
            .filter(|e| e.topic == "search.changed"
                && e.data["id"] == serde_json::json!(taken.id)
                && e.data["doc"].is_null())
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.topic == "ask.answer.deleted" && e.data["question_id"] == serde_json::json!(taken.id))
            .count(),
        1,
        "the second delivery found nothing to do"
    );

    // Staff can still see what it was and who wrote it.
    let seen: InspectQuestionResponse = ctx
        .call(
            MODERATION,
            "InspectQuestion",
            &moderator,
            &InspectQuestionRequest { question_id: taken.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(seen.removed_at.is_set());
    assert_eq!(seen.record.as_option().unwrap().sender.as_option().unwrap().id, asker.id.to_string());
}

#[tokio::test]
async fn a_deleted_upload_leaves_the_text_and_takes_an_image_only_answer_away() {
    let ctx = start().await;
    let (owner, asker) = (ctx.user("Owner").await, ctx.user("Asker").await);
    ctx.open_box(&owner, true).await;
    ctx.update_settings(&owner, UpdateSettingsRequest { allow_images: Some(true), ..Default::default() }).await;
    let core = &ctx.app.core;
    let image_only = ctx.ask(&asker, &owner, "写真だけで").await;
    let with_text = ctx.ask(&asker, &owner, "写真と文字で").await;
    let (first, second, third) = (
        ready_image(core, owner.id, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await.unwrap(),
        ready_image(core, owner.id, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await.unwrap(),
        ready_image(core, asker.id, AssetPurpose::ASSET_PURPOSE_CHAT_IMAGE).await.unwrap(),
    );
    let answer = |question: &Question, text: &str, image: Uuid| AnswerQuestionRequest {
        question_id: question.id.clone(),
        text: text.to_owned(),
        image_asset_id: image.to_string(),
        ..Default::default()
    };
    let _: AnswerQuestionResponse =
        ctx.call(BOX, "AnswerQuestion", &owner, &answer(&image_only, "", first.id)).await.unwrap();
    let _: AnswerQuestionResponse =
        ctx.call(BOX, "AnswerQuestion", &owner, &answer(&with_text, "これ!", second.id)).await.unwrap();
    let pictured = AskQuestionRequest {
        user: owner.id.to_string(),
        text: "この写真どう?".into(),
        signed: true,
        image_asset_id: third.id.to_string(),
        ..Default::default()
    };
    let pictured: AskQuestionResponse = ctx.call(BOX, "AskQuestion", &asker, &pictured).await.unwrap();
    ctx.deliver().await;

    let deleted =
        |asset: Uuid, owner: Uuid| serde_json::json!({ "asset_id": asset, "owner_id": owner, "purpose": 8, "kind": 1 });
    for asset in [first.id, second.id] {
        ctx.announce("media.asset.deleted", deleted(asset, owner.id)).await;
    }
    ctx.announce("media.asset.deleted", deleted(third.id, asker.id)).await;
    let events = ctx.deliver().await;

    // Nothing was left of the first answer: the question is unanswered again.
    let back: GetQuestionResponse = ctx
        .call(
            BOX,
            "GetQuestion",
            &owner,
            &GetQuestionRequest { question_id: image_only.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let back = back.question.as_option().unwrap();
    assert!(back.answer.is_unset());
    assert_eq!(back.folder, QuestionFolder::QUESTION_FOLDER_INBOX);
    assert!(
        events
            .iter()
            .any(|e| e.topic == "ask.answer.deleted" && e.data["question_id"] == serde_json::json!(image_only.id))
    );
    // The second keeps its text.
    let kept: GetQuestionResponse = ctx
        .call(
            BOX,
            "GetQuestion",
            &owner,
            &GetQuestionRequest { question_id: with_text.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let kept = kept.question.as_option().unwrap().answer.as_option().unwrap().clone();
    assert!(kept.image.is_unset());
    assert_eq!(kept.text, "これ!");
    // And the question loses its picture.
    let inbox = ctx.inbox(&owner, QuestionFolder::QUESTION_FOLDER_INBOX).await;
    let question = inbox.iter().find(|q| q.id == pictured.question.as_option().unwrap().id).unwrap();
    assert!(question.image.is_unset());
    assert_eq!(question.text, "この写真どう?");
}

#[tokio::test]
async fn the_purge_counts_from_the_day_something_was_removed() {
    let ctx = start().await;
    let (owner, asker) = (ctx.user("Owner").await, ctx.user("Asker").await);
    ctx.befriend(&owner, &asker).await;
    ctx.crowd(&owner).await;
    ctx.open_box(&owner, true).await;
    ctx.update_settings(&owner, UpdateSettingsRequest { allow_images: Some(true), ..Default::default() }).await;
    let db = &ctx.app.core.db;
    let exists = |table: &'static str, id: &str| {
        let (db, id) = (db.clone(), Uuid::parse_str(id).unwrap());
        async move {
            let found: bool =
                sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE id = $1)")))
                    .bind(id)
                    .fetch_one(&db)
                    .await
                    .unwrap();
            found
        }
    };

    // A question that sat in the box for most of a year and was deleted
    // today, and a response (with a picture) deleted today too.
    let question = ctx.ask(&asker, &owner, "ずっと前の質問").await;
    let prompt = post_prompt(&ctx, &owner, "写真で答えてね").await;
    let picture = ready_image(&ctx.app.core, asker.id, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await.unwrap();
    let request = RespondToPromptRequest {
        prompt_id: prompt.id.clone(),
        text: "これ!".into(),
        image_asset_id: picture.id.to_string(),
        ..Default::default()
    };
    let response: RespondToPromptResponse = ctx.call(PROMPTS, "RespondToPrompt", &asker, &request).await.unwrap();
    let response = response.response.as_option().unwrap().clone();
    ctx.deliver().await;
    for table in ["ask_questions", "ask_prompt_responses"] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE {table} SET created_at = created_at - interval '300 days' WHERE owner_id = $1"
        )))
        .bind(owner.id)
        .execute(db)
        .await
        .unwrap();
    }
    let _: DeleteQuestionResponse = ctx
        .call(
            BOX,
            "DeleteQuestion",
            &owner,
            &DeleteQuestionRequest { question_id: question.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let _: DeleteResponseResponse = ctx
        .call(
            PROMPTS,
            "DeleteResponse",
            &owner,
            &DeleteResponseRequest { response_id: response.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();

    // Moderators may still need them: a report filed today is about them.
    ctx.ask.purge().await.unwrap();
    assert!(exists("ask_questions", &question.id).await, "kept for the retention after its removal, however old it is");
    assert!(!ctx.app.subject_deleted(tokuly_proto::common::SubjectType::SUBJECT_TYPE_QUESTION, &question.id).await);
    assert!(exists("ask_prompt_responses", &response.id).await);
    assert!(tokuly_media::assets::is_attached(&ctx.app.core, picture.id).await.unwrap());

    // Half a year after the removal they go, and the picture is let go of.
    for table in ["ask_questions", "ask_prompt_responses"] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE {table} SET removed_at = removed_at - interval '181 days' WHERE owner_id = $1"
        )))
        .bind(owner.id)
        .execute(db)
        .await
        .unwrap();
    }
    ctx.ask.purge().await.unwrap();
    assert!(!exists("ask_questions", &question.id).await);
    assert!(!exists("ask_prompt_responses", &response.id).await);
    for gone in [&question.id, &response.id] {
        assert!(ctx.app.subject_deleted(tokuly_proto::common::SubjectType::SUBJECT_TYPE_QUESTION, gone).await);
    }
    assert!(!tokuly_media::assets::is_attached(&ctx.app.core, picture.id).await.unwrap());

    // A deleted prompt takes its responses along once it is purged.
    let kept = RespondToPromptRequest {
        prompt_id: prompt.id.clone(),
        text: "のこってた回答".into(),
        signed: true,
        ..Default::default()
    };
    let other = ctx.user("Other").await;
    ctx.befriend(&owner, &other).await;
    let kept: RespondToPromptResponse = ctx.call(PROMPTS, "RespondToPrompt", &other, &kept).await.unwrap();
    let kept = kept.response.as_option().unwrap().clone();
    ctx.deliver().await;
    let _: DeletePromptResponse = ctx
        .call(
            PROMPTS,
            "DeletePrompt",
            &owner,
            &DeletePromptRequest { prompt_id: prompt.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    ctx.ask.purge().await.unwrap();
    assert!(exists("ask_prompts", &prompt.id).await && exists("ask_prompt_responses", &kept.id).await);
    sqlx::query("UPDATE ask_prompts SET removed_at = removed_at - interval '181 days' WHERE id = $1")
        .bind(Uuid::parse_str(&prompt.id).unwrap())
        .execute(db)
        .await
        .unwrap();
    ctx.ask.purge().await.unwrap();
    assert!(!exists("ask_prompts", &prompt.id).await && !exists("ask_prompt_responses", &kept.id).await);
}

#[tokio::test]
async fn the_export_gives_each_side_what_they_can_see_and_no_more() {
    let ctx = start().await;
    let (owner, sender) = (ctx.user("Owner").await, ctx.user(SENDER).await);
    ctx.open_box(&owner, true).await;
    let first = ctx.ask(&sender, &owner, "一つ目").await;
    ctx.block_sender_of_question(&owner, &first.id).await;
    ctx.ask(&sender, &owner, "届かなかった二つ目").await;

    let exporter = ctx
        .app
        .core
        .exporters
        .all()
        .into_iter()
        .find(|exporter| exporter.name() == "ask")
        .expect("the ask section is registered");

    // The owner: what reached them, without its sender; not what was dropped.
    let mine = exporter.export(owner.id).await.unwrap();
    assert_no_trace_of(&sender, SENDER, &mine, "the owner's export");
    let received = mine["questions_received"].as_array().unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(
        (received[0]["text"].as_str(), received[0]["anonymous"].as_bool(), received[0]["asker_id"].is_null()),
        (Some("一つ目"), Some(true), true)
    );
    assert_eq!(mine["sender_blocks"].as_array().unwrap().len(), 1);
    assert_eq!(mine["settings"][0]["box_open"], serde_json::json!(true));

    // The sender: both questions, alike. Nothing tells the dropped one apart.
    let theirs = exporter.export(sender.id).await.unwrap();
    let sent = theirs["questions_sent"].as_array().unwrap();
    assert_eq!(sent.len(), 2);
    let without_identity = |question: &serde_json::Value| {
        let mut question = question.clone();
        for key in ["id", "text", "sent_at"] {
            question.as_object_mut().unwrap().remove(key);
        }
        question
    };
    assert_eq!(without_identity(&sent[0]), without_identity(&sent[1]));
    assert_eq!(sent[1]["answered"], serde_json::json!(false));
    assert!(theirs["questions_received"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn an_open_box_shows_on_the_profile() {
    let ctx = start().await;
    let (owner, friend, stranger) = (ctx.user("Owner").await, ctx.user("Friend").await, ctx.user("Stranger").await);
    ctx.befriend(&owner, &friend).await;
    assert!(
        cards(&ctx, Some(&friend), &owner).await.iter().all(|card| card.kind != "ask.box"),
        "a closed box is not advertised"
    );

    ctx.update_settings(
        &owner,
        UpdateSettingsRequest {
            box_open: Some(true), box_prompt: Some("なんでもどうぞ".into()), ..Default::default()
        },
    )
    .await;
    let for_friend = cards(&ctx, Some(&friend), &owner).await;
    let card = for_friend.iter().find(|card| card.kind == "ask.box").expect("the box card");
    assert_eq!(card.title.as_option().unwrap().ja, "質問箱");
    assert_eq!(card.text.as_option().unwrap().ja, "なんでもどうぞ");
    assert_eq!(card.path, format!("/u/{}/ask", owner.handle));
    assert_eq!(
        (card.data["audience"].as_str(), card.data["can_ask"].as_str(), card.data["answer_count"].as_str()),
        ("friends", "true", "0")
    );

    let for_stranger = cards(&ctx, Some(&stranger), &owner).await;
    assert_eq!(for_stranger.iter().find(|card| card.kind == "ask.box").unwrap().data["can_ask"], "false");
    let for_guest = cards(&ctx, None, &owner).await;
    assert_eq!(for_guest.iter().find(|card| card.kind == "ask.box").unwrap().data["can_ask"], "false");
}

#[tokio::test]
async fn guests_and_ordinary_users_are_refused_where_they_should_be() {
    let ctx = start().await;
    let user = ctx.user("User").await;
    let nothing = serde_json::json!({});

    // Everything but the three public pages needs a signed-in caller.
    let signed_in_only: [(&str, &[&str]); 4] = [
        (
            BOX,
            &[
                "GetSettings",
                "UpdateSettings",
                "AskQuestion",
                "ListAskedQuestions",
                "WithdrawQuestion",
                "ListInbox",
                "GetInboxCounts",
                "MarkQuestionsRead",
                "MoveQuestion",
                "DeleteQuestion",
                "AnswerQuestion",
                "UpdateAnswer",
                "DeleteAnswer",
                "BlockSender",
                "ListSenderBlocks",
                "UnblockSender",
            ],
        ),
        (
            PROMPTS,
            &[
                "CreatePrompt",
                "ClosePrompt",
                "DeletePrompt",
                "GetPrompt",
                "ListPrompts",
                "ListFriendPrompts",
                "RespondToPrompt",
                "ListPromptResponses",
                "ListPublishedResponses",
                "SetResponsePublished",
                "ReleaseResponse",
                "DeleteResponse",
            ],
        ),
        (
            POLLS,
            &[
                "GetRound",
                "Vote",
                "SkipPoll",
                "ShufflePoll",
                "ListFlames",
                "GetFlameSummary",
                "MarkFlamesSeen",
                "BuyHint",
                "GetTopFlames",
            ],
        ),
        (MODERATION, STAFF_ONLY),
    ];
    for (service, methods) in signed_in_only {
        for method in methods {
            let refused =
                ctx.app.call::<_, serde_json::Value>(&format!("{service}/{method}"), None, &nothing).await.unwrap_err();
            assert_eq!(refused.code, "unauthenticated", "{service}/{method} for a guest");
        }
    }
    // The public pages answer a guest (here: that there is no such thing).
    for (method, reason) in
        [("GetBox", "BOX_NOT_FOUND"), ("ListAnswers", "BOX_NOT_FOUND"), ("GetQuestion", "INVALID_ID")]
    {
        let answered =
            ctx.app.call::<_, serde_json::Value>(&format!("{BOX}/{method}"), None, &nothing).await.unwrap_err();
        assert_eq!(answered.reason, reason, "{method} for a guest");
    }
    // Staff tools refuse everyone who is not staff, before looking at the request.
    for method in STAFF_ONLY {
        let refused = ctx
            .app
            .call::<_, serde_json::Value>(&format!("{MODERATION}/{method}"), Some(&user), &nothing)
            .await
            .unwrap_err();
        assert_eq!(
            (refused.code.as_str(), refused.reason.as_str()),
            ("permission_denied", "FORBIDDEN"),
            "{method} for an ordinary user"
        );
    }
}

/// Every RPC of `AskModerationService`.
const STAFF_ONLY: &[&str] = &[
    "InspectQuestion",
    "InspectPromptResponse",
    "InspectFlame",
    "ListPollQuestions",
    "AddPollQuestion",
    "SetPollQuestionEnabled",
];
