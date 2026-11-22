//! What becomes of a sender's account shows on nothing they sent anonymously.
//!
//! If an anonymous question, response or flame appeared, disappeared or
//! changed when someone was suspended or deleted their account, whoever
//! noticed both would know who sent it. So:
//!
//! - a suspension hides only what carries the sender's name, and stops what
//!   has not been delivered yet;
//! - a deleted account leaves what it sent as it was, without the name;
//! - a picture on an anonymous item is as anonymous as its text.
mod common;

use common::*;
use serde_json::{Value, json};
use tokuly_media::testing::ready_image;
use tokuly_proto::{ask::*, media::AssetPurpose};
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

/// A name no response can contain by accident.
const SENDER: &str = "Zebedee Quillfeather";

async fn post_prompt(ctx: &Ctx, owner: &TestUser, text: &str) -> Prompt {
    let response: CreatePromptResponse = ctx
        .call(PROMPTS, "CreatePrompt", owner, &CreatePromptRequest { text: text.to_owned(), ..Default::default() })
        .await
        .expect("create prompt");
    ctx.deliver().await;
    response.prompt.as_option().cloned().expect("prompt")
}

async fn respond(
    ctx: &Ctx,
    sender: &TestUser,
    prompt: &Prompt,
    text: &str,
    signed: bool,
    image: Option<Uuid>,
) -> Result<PromptResponse, RpcError> {
    let request = RespondToPromptRequest {
        prompt_id: prompt.id.clone(),
        text: text.to_owned(),
        signed,
        image_asset_id: image.map(|id| id.to_string()).unwrap_or_default(),
        ..Default::default()
    };
    let response: RespondToPromptResponse = ctx.call(PROMPTS, "RespondToPrompt", sender, &request).await?;
    ctx.deliver().await;
    Ok(response.response.as_option().cloned().expect("response"))
}

async fn publish(ctx: &Ctx, owner: &TestUser, response: &PromptResponse) {
    let request =
        SetResponsePublishedRequest { response_id: response.id.clone(), published: true, ..Default::default() };
    let _: SetResponsePublishedResponse =
        ctx.call(PROMPTS, "SetResponsePublished", owner, &request).await.expect("publish");
}

/// The items of a list that are anonymous (all of them when `only_anonymous`
/// is off). Proto JSON leaves `anonymous` out when it is false.
fn kept(list: &Value, only_anonymous: bool) -> Vec<Value> {
    list.as_array()
        .map(|items| items.iter().filter(|item| !only_anonymous || item["anonymous"] == json!(true)).cloned().collect())
        .unwrap_or_default()
}

/// Everything `owner` is shown of what was sent to them, as it goes over
/// the wire, with what a friend reads on their pages and what their data
/// export says.
async fn view(ctx: &Ctx, owner: &TestUser, friend: &TestUser, prompt: &Prompt, only_anonymous: bool) -> Value {
    let answered = QuestionFolder::QUESTION_FOLDER_ANSWERED;
    let inbox = ctx.json(BOX, "ListInbox", owner, &ListInboxRequest::default()).await;
    let answers =
        ctx.json(BOX, "ListInbox", owner, &ListInboxRequest { folder: answered.into(), ..Default::default() }).await;
    let page = ctx
        .json(BOX, "ListAnswers", friend, &ListAnswersRequest { user: owner.id.to_string(), ..Default::default() })
        .await;
    let responses = ctx
        .json(
            PROMPTS,
            "ListPromptResponses",
            owner,
            &ListPromptResponsesRequest { prompt_id: prompt.id.clone(), ..Default::default() },
        )
        .await;
    let published = ctx
        .json(
            PROMPTS,
            "ListPublishedResponses",
            friend,
            &ListPublishedResponsesRequest { prompt_id: prompt.id.clone(), ..Default::default() },
        )
        .await;
    let exporter =
        ctx.app.core.exporters.all().into_iter().find(|exporter| exporter.name() == "ask").expect("the ask exporter");
    let export = exporter.export(owner.id).await.expect("export");
    let exported_responses: Vec<Value> = export["prompts"]
        .as_array()
        .expect("prompts")
        .iter()
        .flat_map(|prompt| kept(&prompt["responses"], only_anonymous))
        .collect();
    json!({
        "inbox": kept(&inbox["questions"], only_anonymous),
        "answered": kept(&answers["questions"], only_anonymous),
        "page": kept(&page["questions"], only_anonymous),
        "responses": kept(&responses["responses"], only_anonymous),
        "published": kept(&published["responses"], only_anonymous),
        "flames": ctx.json(POLLS, "ListFlames", owner, &ListFlamesRequest::default()).await,
        "flame_summary": ctx.json(POLLS, "GetFlameSummary", owner, &GetFlameSummaryRequest::default()).await,
        "ranking": ctx.json(POLLS, "GetTopFlames", owner, &GetTopFlamesRequest::default()).await,
        "exported_questions": kept(&export["questions_received"], only_anonymous),
        "exported_responses": exported_responses,
        "exported_flames": export["flames_received"],
    })
}

fn ids(list: &Value) -> Vec<String> {
    list.as_array().expect("a list").iter().map(|item| item["id"].as_str().expect("an id").to_owned()).collect()
}

#[tokio::test]
async fn a_suspension_changes_nothing_a_recipient_sees_of_anonymous_items() {
    let ctx = start().await;
    let (owner, sender, other) = (ctx.user("Owner").await, ctx.user(SENDER).await, ctx.user("Other").await);
    ctx.befriend(&owner, &sender).await;
    let crowd = ctx.crowd(&owner).await;
    let friend = &crowd[0];
    ctx.friends_of(&sender, 3).await;
    ctx.open_box(&owner, true).await;

    // Anonymous: a waiting question, an answered one, a published prompt
    // response and a flame. Under the sender's name: the same.
    let waiting = ctx.ask(&sender, &owner, "匿名でまってる質問").await;
    let answered = ctx.ask(&sender, &owner, "匿名で答えてもらった質問").await;
    ctx.answer(&owner, &answered.id, "こたえ", AnswerAudience::ANSWER_AUDIENCE_PUBLIC).await;
    let signed_waiting = ctx.try_ask(&sender, &owner, "名前つきでまってる質問", true).await.unwrap();
    let signed_answered = ctx.try_ask(&sender, &owner, "名前つきで答えてもらった質問", true).await.unwrap();
    ctx.deliver().await;
    ctx.answer(&owner, &signed_answered.id, "こたえ", AnswerAudience::ANSWER_AUDIENCE_PUBLIC).await;
    let from_other = ctx.ask(&other, &owner, "ほかの人の質問").await;
    let prompt = post_prompt(&ctx, &owner, "私の第一印象は?").await;
    let signed_prompt = post_prompt(&ctx, &owner, "名前つきで答えてね").await;
    let response = respond(&ctx, &sender, &prompt, "匿名の回答", false, None).await.unwrap();
    publish(&ctx, &owner, &response).await;
    let signed_response = respond(&ctx, &sender, &signed_prompt, "名前つきの回答", true, None).await.unwrap();
    ctx.vote_for(&sender, &owner).await;

    // The first look marks things as seen; the second is the picture to keep.
    view(&ctx, &owner, friend, &prompt, true).await;
    let before = view(&ctx, &owner, friend, &prompt, true).await;
    assert_eq!(ids(&before["inbox"]), vec![from_other.id.clone(), waiting.id.clone()]);
    assert_eq!(ids(&before["answered"]), vec![answered.id.clone()]);
    assert_eq!(ids(&before["page"]), vec![answered.id.clone()]);
    assert_eq!(ids(&before["responses"]), vec![response.id.clone()]);
    assert_eq!(ids(&before["published"]), vec![response.id.clone()]);
    assert_eq!(before["flames"]["flames"].as_array().unwrap().len(), 1);
    assert_eq!(before["ranking"]["myCount"], json!(1));
    assert_eq!(ids(&before["exported_questions"]).len(), 3);
    assert_no_trace_of(&sender, SENDER, &before, "what the owner sees of anonymous items");

    // Something still on its way when the account is suspended.
    let on_its_way = ctx.try_ask(&sender, &owner, "まだ届いてない質問", false).await.unwrap();
    let notices_before = ctx.all_notifications_for(owner.id).await.len();
    let events = ctx.suspend(&sender).await;

    // Everything anonymous is exactly where and what it was.
    assert_eq!(view(&ctx, &owner, friend, &prompt, true).await, before, "the suspension shows on something anonymous");
    assert!(
        !events.iter().any(|e| e.topic == "search.changed" && e.data["id"] == json!(answered.id)),
        "the answer to an anonymous question stays in the search index"
    );
    // What carries the sender's name is hidden for as long as it lasts.
    let inbox = ctx.inbox(&owner, QuestionFolder::QUESTION_FOLDER_INBOX).await;
    assert!(!inbox.iter().any(|q| q.id == signed_waiting.id), "a signed question of a suspended account is hidden");
    let page: ListAnswersResponse = ctx
        .call(BOX, "ListAnswers", &other, &ListAnswersRequest { user: owner.id.to_string(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(page.questions.iter().map(|q| q.id.as_str()).collect::<Vec<_>>(), vec![answered.id.as_str()]);
    assert!(
        events.iter().any(|e| e.topic == "search.changed"
            && e.data["id"] == json!(signed_answered.id)
            && e.data["doc"].is_null()),
        "the answer to a signed question leaves the search index"
    );
    let listing = ListPromptResponsesRequest { prompt_id: signed_prompt.id.clone(), ..Default::default() };
    let signed_responses: ListPromptResponsesResponse =
        ctx.call(PROMPTS, "ListPromptResponses", &owner, &listing).await.unwrap();
    assert!(signed_responses.responses.is_empty(), "a signed response of a suspended account is hidden");
    // What was not delivered yet never is.
    assert!(!inbox.iter().any(|q| q.id == on_its_way.id));
    assert_eq!(ctx.all_notifications_for(owner.id).await.len(), notices_before, "and nobody is told about it");

    // The end of the suspension brings back the signed items and changes
    // nothing else.
    let events = ctx.reinstate(&sender).await;
    assert_eq!(view(&ctx, &owner, friend, &prompt, true).await, before);
    let inbox = ctx.inbox(&owner, QuestionFolder::QUESTION_FOLDER_INBOX).await;
    assert!(inbox.iter().any(|q| q.id == signed_waiting.id));
    assert!(!inbox.iter().any(|q| q.id == on_its_way.id), "what was stopped stays stopped");
    assert!(
        events.iter().any(|e| e.topic == "search.changed"
            && e.data["id"] == json!(signed_answered.id)
            && !e.data["doc"].is_null())
    );
    let signed_responses: ListPromptResponsesResponse =
        ctx.call(PROMPTS, "ListPromptResponses", &owner, &listing).await.unwrap();
    assert_eq!(
        signed_responses.responses.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec![signed_response.id.as_str()]
    );
    // The sender sees the stopped question like any unanswered one.
    // (The suspension ended their session; they sign in again.)
    let sender = ctx.app.sign_in(sender.id).await;
    let stopped = ctx.asked(&sender).await.into_iter().find(|q| q.id == on_its_way.id).expect("still in their list");
    assert_eq!(stopped.sent_state, SentState::SENT_STATE_WAITING);
}

#[tokio::test]
async fn a_deleted_account_leaves_what_it_sent_anonymously_exactly_as_it_was() {
    let ctx = start().await;
    let (owner, sender) = (ctx.user("Owner").await, ctx.user(SENDER).await);
    ctx.befriend(&owner, &sender).await;
    let crowd = ctx.crowd(&owner).await;
    let friend = &crowd[0];
    ctx.friends_of(&sender, 3).await;
    ctx.open_box(&owner, true).await;
    ctx.update_settings(&owner, UpdateSettingsRequest { allow_images: Some(true), ..Default::default() }).await;
    let core = &ctx.app.core;

    // A question and a prompt response, each with a picture, and a flame.
    let picture = ready_image(core, sender.id, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await.unwrap();
    let pictured = AskQuestionRequest {
        user: owner.id.to_string(),
        text: "この写真どう?".into(),
        image_asset_id: picture.id.to_string(),
        ..Default::default()
    };
    let _: AskQuestionResponse = ctx.call(BOX, "AskQuestion", &sender, &pictured).await.unwrap();
    ctx.deliver().await;
    let answered = ctx.ask(&sender, &owner, "答えてもらった質問").await;
    ctx.answer(&owner, &answered.id, "こたえ", AnswerAudience::ANSWER_AUDIENCE_PUBLIC).await;
    let prompt = post_prompt(&ctx, &owner, "写真で答えてね").await;
    let second_picture = ready_image(core, sender.id, AssetPurpose::ASSET_PURPOSE_CHAT_IMAGE).await.unwrap();
    let response = respond(&ctx, &sender, &prompt, "これ!", false, Some(second_picture.id)).await.unwrap();
    publish(&ctx, &owner, &response).await;
    ctx.vote_for(&sender, &owner).await;

    view(&ctx, &owner, friend, &prompt, false).await;
    let before = view(&ctx, &owner, friend, &prompt, false).await;
    assert_eq!(before["inbox"][0]["image"]["assetId"], json!(picture.id.to_string()));
    assert_eq!(before["responses"][0]["image"]["assetId"], json!(second_picture.id.to_string()));
    assert_no_trace_of(&sender, SENDER, &before, "what the owner sees");

    for _ in 0..2 {
        ctx.announce("identity.user.deleted", json!({ "user_id": sender.id })).await;
    }

    // Text, pictures, flames and what they sell: all as before. A picture
    // that left with the account would mark its question.
    assert_eq!(view(&ctx, &owner, friend, &prompt, false).await, before);
    for asset in [picture.id, second_picture.id] {
        assert!(
            tokuly_media::assets::is_attached(core, asset).await.unwrap(),
            "the item keeps holding its picture, so the media domain keeps it"
        );
    }
    let left: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM ask_questions WHERE sender_id = $1)
              + (SELECT count(*) FROM ask_prompt_responses WHERE sender_id = $1)
              + (SELECT count(*) FROM ask_flames WHERE sender_id = $1)",
    )
    .bind(sender.id)
    .fetch_one(&core.db)
    .await
    .unwrap();
    assert_eq!(left, 0, "nothing points back at the deleted account");
}

#[tokio::test]
async fn an_anonymous_question_can_carry_a_picture_uploaded_for_it() {
    let ctx = start().await;
    let (owner, sender, other) = (ctx.user("Owner").await, ctx.user(SENDER).await, ctx.user("Other").await);
    ctx.open_box(&owner, true).await;
    let core = &ctx.app.core;
    let picture = ready_image(core, sender.id, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await.unwrap();
    let with = |image: Uuid, signed: bool| AskQuestionRequest {
        user: owner.id.to_string(),
        text: "これ見て".into(),
        signed,
        image_asset_id: image.to_string(),
        ..Default::default()
    };
    let ask = |who: &TestUser, image: Uuid, signed: bool| {
        let (ctx, who, request) = (&ctx, who.clone(), with(image, signed));
        async move { ctx.call::<_, AskQuestionResponse>(BOX, "AskQuestion", &who, &request).await }
    };

    // The owner decides whether the box takes pictures at all.
    for signed in [false, true] {
        assert_eq!(ask(&sender, picture.id, signed).await.unwrap_err().reason, "IMAGES_NOT_ALLOWED");
    }
    ctx.update_settings(&owner, UpdateSettingsRequest { allow_images: Some(true), ..Default::default() }).await;
    assert_eq!(ask(&other, picture.id, false).await.unwrap_err().reason, "ASSET_NOT_FOUND", "someone else's upload");

    // An anonymous question with a picture: the owner gets both, and
    // nothing about either says who sent them.
    let sent = ask(&sender, picture.id, false).await.unwrap();
    let sent = sent.question.as_option().unwrap().clone();
    assert!(sent.anonymous);
    assert_eq!(sent.image.as_option().expect("the picture").asset_id, picture.id.to_string());
    for event in ctx.deliver().await {
        assert_ne!(event.actor_id, Some(sender.id));
        assert_no_trace_of(&sender, SENDER, &event.data, &format!("the {} event", event.topic));
    }
    let inbox = ctx.json(BOX, "ListInbox", &owner, &ListInboxRequest::default()).await;
    assert_eq!(inbox["questions"][0]["image"]["assetId"], json!(picture.id.to_string()));
    assert_no_trace_of(&sender, SENDER, &inbox, "the inbox with the picture");
    assert!(tokuly_media::assets::is_attached(core, picture.id).await.unwrap(), "the question holds the upload");
    // What the media domain tells anyone but the uploader about the upload.
    let public = tokuly_media::assets::find(core, picture.id).await.unwrap().unwrap().to_public_proto();
    assert_no_trace_of(&sender, SENDER, &public, "the public view of the upload");
    assert!(public.owner_id.is_empty() && public.file_name.is_empty());

    // The same upload a second time would tie two questions to one sender:
    // two anonymous ones, or an anonymous one and one under their name.
    for signed in [false, true] {
        assert_eq!(ask(&sender, picture.id, signed).await.unwrap_err().reason, "IMAGE_ALREADY_USED");
    }
    let fresh = ready_image(core, sender.id, AssetPurpose::ASSET_PURPOSE_CHAT_IMAGE).await.unwrap();
    let signed = ask(&sender, fresh.id, true).await.unwrap();
    assert!(!signed.question.as_option().unwrap().anonymous);
    assert_eq!(ask(&sender, fresh.id, false).await.unwrap_err().reason, "IMAGE_ALREADY_USED");
}

#[tokio::test]
async fn a_prompt_response_can_carry_a_picture() {
    let ctx = start().await;
    let (owner, sender) = (ctx.user("Owner").await, ctx.user(SENDER).await);
    ctx.befriend(&owner, &sender).await;
    ctx.crowd(&owner).await;
    let core = &ctx.app.core;
    let picture = ready_image(core, sender.id, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await.unwrap();
    let first = post_prompt(&ctx, &owner, "写真で答えてね").await;
    let second = post_prompt(&ctx, &owner, "もう一枚!").await;

    assert_eq!(
        respond(&ctx, &sender, &first, "これ!", false, Some(picture.id)).await.unwrap_err().reason,
        "IMAGES_NOT_ALLOWED",
        "pictures are off until the owner turns them on"
    );
    ctx.update_settings(&owner, UpdateSettingsRequest { allow_images: Some(true), ..Default::default() }).await;
    let sent = respond(&ctx, &sender, &first, "これ!", false, Some(picture.id)).await.unwrap();
    assert!(sent.anonymous);
    assert_eq!(sent.image.as_option().expect("the picture").asset_id, picture.id.to_string());
    let received = ctx
        .json(
            PROMPTS,
            "ListPromptResponses",
            &owner,
            &ListPromptResponsesRequest { prompt_id: first.id.clone(), ..Default::default() },
        )
        .await;
    assert_eq!(received["responses"][0]["image"]["assetId"], json!(picture.id.to_string()));
    assert_no_trace_of(&sender, SENDER, &received, "the response with its picture");
    assert_eq!(
        respond(&ctx, &sender, &second, "おなじ写真", false, Some(picture.id)).await.unwrap_err().reason,
        "IMAGE_ALREADY_USED"
    );

    // Moderators see the picture with the response; a takedown lets go of it.
    let moderator = ctx.app.user_with_roles("Moderator", &["moderator"]).await;
    let request = InspectPromptResponseRequest { response_id: sent.id.clone(), ..Default::default() };
    let seen: InspectPromptResponseResponse =
        ctx.call(MODERATION, "InspectPromptResponse", &moderator, &request).await.unwrap();
    assert_eq!(seen.image.as_option().expect("the picture").asset_id, picture.id.to_string());
    ctx.announce(
        "admin.content.removed",
        json!({ "subject_type": tokuly_proto::common::SubjectType::SUBJECT_TYPE_QUESTION as i32, "subject_id": sent.id, "reason": "harassment", "moderator_id": moderator.id }),
    )
    .await;
    assert!(!tokuly_media::assets::is_attached(core, picture.id).await.unwrap());
}
