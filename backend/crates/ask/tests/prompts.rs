//! `PromptService`: prompts, responses, and what the owner publishes.
mod common;

use buffa::EnumValue;
use common::*;
use tokuly_core::events::Event;
use tokuly_proto::{
    ask::{ask_event::Event as Push, *},
    common::SubjectType,
};
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

/// A name no response can contain by accident.
const RESPONDER: &str = "Zebedee Quillfeather";

async fn post(ctx: &Ctx, owner: &TestUser, text: &str) -> Prompt {
    let response: CreatePromptResponse = ctx
        .call(
            PROMPTS,
            "CreatePrompt",
            owner,
            &CreatePromptRequest { text: text.to_owned(), emoji: "👀".into(), ..Default::default() },
        )
        .await
        .expect("create prompt");
    ctx.deliver().await;
    response.prompt.as_option().cloned().expect("prompt")
}

async fn respond(
    ctx: &Ctx,
    sender: &TestUser,
    prompt_id: &str,
    text: &str,
    signed: bool,
) -> Result<PromptResponse, RpcError> {
    let request =
        RespondToPromptRequest { prompt_id: prompt_id.to_owned(), text: text.to_owned(), signed, ..Default::default() };
    let response: RespondToPromptResponse = ctx.call(PROMPTS, "RespondToPrompt", sender, &request).await?;
    ctx.deliver().await;
    Ok(response.response.as_option().cloned().expect("response"))
}

async fn page_of(ctx: &Ctx, viewer: &TestUser, prompt_id: &str) -> Result<GetPromptResponse, RpcError> {
    ctx.call(PROMPTS, "GetPrompt", viewer, &GetPromptRequest { prompt_id: prompt_id.to_owned(), ..Default::default() })
        .await
}

async fn responses(ctx: &Ctx, owner: &TestUser, prompt_id: &str, filter: ResponseFilter) -> Vec<PromptResponse> {
    let request =
        ListPromptResponsesRequest { prompt_id: prompt_id.to_owned(), filter: filter.into(), ..Default::default() };
    let response: ListPromptResponsesResponse =
        ctx.call(PROMPTS, "ListPromptResponses", owner, &request).await.expect("list responses");
    response.responses
}

fn shape(events: &[Event]) -> Vec<(String, Option<Uuid>, Vec<String>)> {
    events
        .iter()
        .map(|e| {
            (e.topic.clone(), e.actor_id, e.data.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default())
        })
        .collect()
}

#[tokio::test]
async fn a_prompt_is_posted_to_friends_and_closes() {
    let ctx = start().await;
    let (owner, friend, muter, stranger) =
        (ctx.user("Owner").await, ctx.user("Friend").await, ctx.user("Muter").await, ctx.user("Stranger").await);
    ctx.befriend(&owner, &friend).await;
    ctx.befriend(&owner, &muter).await;
    ctx.mute(&muter, &owner).await;

    for (request, reason) in [
        (CreatePromptRequest { text: " ".into(), ..Default::default() }, "PROMPT_TEXT_INVALID"),
        (CreatePromptRequest { text: "あ".repeat(101), ..Default::default() }, "PROMPT_TEXT_INVALID"),
        (CreatePromptRequest { text: "お題".into(), emoji: "abc".into(), ..Default::default() }, "PROMPT_TEXT_INVALID"),
        (
            CreatePromptRequest { text: "お題".into(), duration_seconds: 60, ..Default::default() },
            "PROMPT_DURATION_INVALID",
        ),
        (
            CreatePromptRequest { text: "お題".into(), duration_seconds: 8 * 24 * 3600, ..Default::default() },
            "PROMPT_DURATION_INVALID",
        ),
    ] {
        assert_eq!(
            ctx.call::<_, CreatePromptResponse>(PROMPTS, "CreatePrompt", &owner, &request).await.unwrap_err().reason,
            reason
        );
    }

    ctx.new_events().await;
    let request =
        CreatePromptRequest { text: "  私の第一印象は?  ".into(), emoji: "👀".into(), ..Default::default() };
    let created: CreatePromptResponse = ctx.call(PROMPTS, "CreatePrompt", &owner, &request).await.unwrap();
    let prompt = created.prompt.as_option().unwrap().clone();
    assert_eq!((prompt.text.as_str(), prompt.emoji.as_str()), ("私の第一印象は?", "👀"));
    assert!(prompt.open && !prompt.can_respond && !prompt.responded);
    assert_eq!(prompt.owner.as_option().unwrap().id, owner.id.to_string());
    assert_eq!(prompt.path, format!("/ask/prompts/{}", prompt.id));
    let open_for = prompt.closes_at.as_option().unwrap().seconds - prompt.created_at.as_option().unwrap().seconds;
    assert!((24 * 3600 - 5..=24 * 3600 + 5).contains(&open_for), "a prompt stays open for a day unless told otherwise");

    // Friends are told, except the one who muted the owner.
    let events = ctx.deliver().await;
    let opened = events.iter().find(|e| e.topic == "ask.prompt.opened").expect("ask.prompt.opened");
    assert_eq!(opened.actor_id, Some(owner.id));
    assert_eq!(opened.data["prompt_id"], serde_json::json!(prompt.id));
    let told = ctx.notifications_for(friend.id, "ask.prompt").await;
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].actor_id, Some(owner.id));
    assert_eq!(told[0].data["collapse_key"], serde_json::json!(format!("ask.prompt:{}", owner.id)));
    assert_eq!(told[0].data["body"]["ja"], serde_json::json!("私の第一印象は?"));
    for untold in [&muter, &stranger, &owner] {
        assert!(ctx.notifications_for(untold.id, "ask.prompt").await.is_empty(), "{}", untold.handle);
    }

    // At most three open at a time.
    post(&ctx, &owner, "二つ目").await;
    let third = post(&ctx, &owner, "三つ目").await;
    let fourth = CreatePromptRequest { text: "四つ目".into(), ..Default::default() };
    assert_eq!(
        ctx.call::<_, CreatePromptResponse>(PROMPTS, "CreatePrompt", &owner, &fourth).await.unwrap_err().reason,
        "TOO_MANY_OPEN_PROMPTS"
    );

    // Closing early.
    let close = ClosePromptRequest { prompt_id: third.id.clone(), ..Default::default() };
    assert_eq!(
        ctx.call::<_, ClosePromptResponse>(PROMPTS, "ClosePrompt", &friend, &close).await.unwrap_err().reason,
        "PROMPT_NOT_FOUND"
    );
    let closed: ClosePromptResponse = ctx.call(PROMPTS, "ClosePrompt", &owner, &close).await.unwrap();
    assert!(!closed.prompt.as_option().unwrap().open);
    assert_eq!(respond(&ctx, &friend, &third.id, "おそかった", false).await.unwrap_err().reason, "PROMPT_CLOSED");
    let _: CreatePromptResponse = ctx.call(PROMPTS, "CreatePrompt", &owner, &fourth).await.unwrap();

    // Deleting.
    let delete = DeletePromptRequest { prompt_id: prompt.id.clone(), ..Default::default() };
    assert_eq!(
        ctx.call::<_, DeletePromptResponse>(PROMPTS, "DeletePrompt", &friend, &delete).await.unwrap_err().reason,
        "PROMPT_NOT_FOUND"
    );
    let _: DeletePromptResponse = ctx.call(PROMPTS, "DeletePrompt", &owner, &delete).await.unwrap();
    assert_eq!(page_of(&ctx, &owner, &prompt.id).await.unwrap_err().reason, "PROMPT_NOT_FOUND");
    assert_eq!(page_of(&ctx, &friend, &prompt.id).await.unwrap_err().reason, "PROMPT_NOT_FOUND");
    assert_eq!(
        ctx.call::<_, DeletePromptResponse>(PROMPTS, "DeletePrompt", &owner, &delete).await.unwrap_err().reason,
        "PROMPT_NOT_FOUND"
    );
}

#[tokio::test]
async fn friends_respond_once_and_an_anonymous_response_names_nobody() {
    let ctx = start().await;
    let (owner, anonymous, signer, stranger) =
        (ctx.user("Owner").await, ctx.user(RESPONDER).await, ctx.user("Signer").await, ctx.user("Stranger").await);
    ctx.befriend(&owner, &anonymous).await;
    ctx.befriend(&owner, &signer).await;
    let prompt = post(&ctx, &owner, "私の第一印象は?").await;

    // A prompt is for friends.
    assert_eq!(page_of(&ctx, &stranger, &prompt.id).await.unwrap_err().reason, "PROMPT_NOT_FOUND");
    assert_eq!(respond(&ctx, &stranger, &prompt.id, "だれ?", false).await.unwrap_err().reason, "PROMPT_NOT_FOUND");
    assert_eq!(respond(&ctx, &owner, &prompt.id, "自分で", false).await.unwrap_err().reason, "CANNOT_RESPOND_TO_OWN");
    for bad in ["", &"あ".repeat(301)] {
        assert_eq!(
            respond(&ctx, &anonymous, &prompt.id, bad, false).await.unwrap_err().reason,
            "RESPONSE_TEXT_INVALID"
        );
    }
    let for_friend = page_of(&ctx, &anonymous, &prompt.id).await.unwrap();
    assert!(for_friend.prompt.as_option().unwrap().can_respond && for_friend.my_response.is_unset());

    // An anonymous response.
    let mut pushes = ctx.app.listen_user(owner.id).await;
    ctx.new_events().await;
    let request =
        RespondToPromptRequest { prompt_id: prompt.id.clone(), text: "やさしそう!".into(), ..Default::default() };
    let sent: RespondToPromptResponse = ctx.call(PROMPTS, "RespondToPrompt", &anonymous, &request).await.unwrap();
    let sent = sent.response.as_option().unwrap().clone();
    assert!(sent.anonymous && sent.sent_by_me && sent.responder.is_unset() && !sent.published);
    for event in ctx.deliver().await {
        assert_ne!(event.actor_id, Some(anonymous.id), "{} names the responder as its actor", event.topic);
        assert_no_trace_of(&anonymous, RESPONDER, &event.data, &format!("the {} event", event.topic));
    }
    match next_push(&mut pushes).await.expect("the owner is pushed the response") {
        Push::PromptResponseReceived(received) => {
            assert_no_trace_of(&anonymous, RESPONDER, &*received, "the realtime push");
            assert_eq!(received.response.as_option().unwrap().id, sent.id);
            let counted = received.prompt.as_option().unwrap();
            assert_eq!((counted.response_count, counted.unseen_count), (1, 1));
        }
        other => panic!("unexpected push {other:?}"),
    }
    let notices = ctx.notifications_for(owner.id, "ask.prompt_response").await;
    assert_eq!(notices.len(), 1);
    assert!(notices[0].actor_id.is_none());
    assert_no_trace_of(&anonymous, RESPONDER, &notices[0].data, "the notification");
    for (rpc, response) in [
        (
            "ListPromptResponses",
            ctx.json(
                PROMPTS,
                "ListPromptResponses",
                &owner,
                &ListPromptResponsesRequest { prompt_id: prompt.id.clone(), ..Default::default() },
            )
            .await,
        ),
        (
            "GetPrompt",
            ctx.json(
                PROMPTS,
                "GetPrompt",
                &owner,
                &GetPromptRequest { prompt_id: prompt.id.clone(), ..Default::default() },
            )
            .await,
        ),
        ("ListPrompts", ctx.json(PROMPTS, "ListPrompts", &owner, &ListPromptsRequest::default()).await),
        (
            "SetResponsePublished",
            ctx.json(
                PROMPTS,
                "SetResponsePublished",
                &owner,
                &SetResponsePublishedRequest { response_id: sent.id.clone(), published: true, ..Default::default() },
            )
            .await,
        ),
        (
            "ListPublishedResponses",
            ctx.json(
                PROMPTS,
                "ListPublishedResponses",
                &signer,
                &ListPublishedResponsesRequest { prompt_id: prompt.id.clone(), ..Default::default() },
            )
            .await,
        ),
    ] {
        assert_no_trace_of(&anonymous, RESPONDER, &response, rpc);
    }

    // Once per person.
    let again = respond(&ctx, &anonymous, &prompt.id, "もう一回", false).await.unwrap_err();
    assert_eq!((again.code.as_str(), again.reason.as_str()), ("already_exists", "ALREADY_RESPONDED"));
    let mine = page_of(&ctx, &anonymous, &prompt.id).await.unwrap();
    assert!(mine.prompt.as_option().unwrap().responded && !mine.prompt.as_option().unwrap().can_respond);
    assert_eq!(mine.my_response.as_option().unwrap().text, "やさしそう!");

    // A signed response names its writer, to the owner and on the page.
    let signed = respond(&ctx, &signer, &prompt.id, "しっかり者!", true).await.unwrap();
    assert!(!signed.anonymous);
    let received = responses(&ctx, &owner, &prompt.id, ResponseFilter::RESPONSE_FILTER_RECEIVED).await;
    assert_eq!(received.len(), 2);
    assert_eq!(received[0].responder.as_option().expect("a signed responder is named").id, signer.id.to_string());
    assert!(received[1].responder.is_unset() && received[1].anonymous);
    let notices = ctx.notifications_for(owner.id, "ask.prompt_response").await;
    assert_eq!(notices.last().unwrap().actor_id, Some(signer.id));
}

#[tokio::test]
async fn the_owner_chooses_what_is_published() {
    let ctx = start().await;
    let (owner, first, second, third) =
        (ctx.user("Owner").await, ctx.user("First").await, ctx.user("Second").await, ctx.user("Third").await);
    for friend in [&first, &second, &third] {
        ctx.befriend(&owner, friend).await;
    }
    ctx.update_settings(
        &owner,
        UpdateSettingsRequest {
            muted_words: MutedWords { words: vec!["こわい".into()], ..Default::default() }.into(),
            ..Default::default()
        },
    )
    .await;
    let prompt = post(&ctx, &owner, "私の第一印象は?").await;
    let kind = respond(&ctx, &first, &prompt.id, "やさしそう", false).await.unwrap();
    let plain = respond(&ctx, &second, &prompt.id, "ふつう", false).await.unwrap();
    let held = respond(&ctx, &third, &prompt.id, "ちょっとこわい", false).await.unwrap();
    assert!(!held.held, "the responder is not told that the filter held their response");

    // The owner's counters, and the held response apart from the others.
    let before = page_of(&ctx, &owner, &prompt.id).await.unwrap();
    let counters = before.prompt.as_option().unwrap();
    assert_eq!(
        (counters.response_count, counters.unseen_count, counters.held_count, counters.published_count),
        (2, 2, 1, 0)
    );
    // One request per delivered response, none for the held one; they share
    // a collapse key, so the owner's inbox shows one entry per prompt.
    let notices = ctx.notifications_for(owner.id, "ask.prompt_response").await;
    assert_eq!(notices.len(), 2);
    assert!(
        notices.iter().all(
            |notice| notice.data["collapse_key"] == serde_json::json!(format!("ask.prompt_response:{}", prompt.id))
        )
    );
    let received = responses(&ctx, &owner, &prompt.id, ResponseFilter::RESPONSE_FILTER_RECEIVED).await;
    assert_eq!(received.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), vec!["ふつう", "やさしそう"]);
    assert!(received.iter().all(|r| !r.seen && !r.held && !r.published));
    assert_eq!(
        page_of(&ctx, &owner, &prompt.id).await.unwrap().prompt.as_option().unwrap().unseen_count,
        0,
        "listing them marks them seen"
    );
    let in_filter = responses(&ctx, &owner, &prompt.id, ResponseFilter::RESPONSE_FILTER_HELD).await;
    assert_eq!(in_filter.len(), 1);
    assert!(in_filter[0].held && in_filter[0].id == held.id);

    // Friends see no counters and nothing unpublished.
    let for_friend = page_of(&ctx, &second, &prompt.id).await.unwrap();
    let shown = for_friend.prompt.as_option().unwrap();
    assert_eq!((shown.response_count, shown.unseen_count, shown.held_count), (0, 0, 0));
    assert!(for_friend.responses.is_empty());

    // Publishing.
    let publish = |id: &str, published: bool| SetResponsePublishedRequest {
        response_id: id.to_owned(),
        published,
        ..Default::default()
    };
    let published: SetResponsePublishedResponse =
        ctx.call(PROMPTS, "SetResponsePublished", &owner, &publish(&kind.id, true)).await.unwrap();
    assert!(
        published.response.as_option().unwrap().published
            && published.response.as_option().unwrap().published_at.is_set()
    );
    let page = page_of(&ctx, &second, &prompt.id).await.unwrap();
    assert_eq!(page.responses.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), vec!["やさしそう"]);
    assert_eq!(page.prompt.as_option().unwrap().published_count, 1);
    assert!(!page.responses[0].sent_by_me && !page.responses[0].seen);
    let listed: ListPublishedResponsesResponse = ctx
        .call(
            PROMPTS,
            "ListPublishedResponses",
            &first,
            &ListPublishedResponsesRequest { prompt_id: prompt.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(listed.responses[0].sent_by_me, "its writer recognises their own response on the page");
    assert_eq!(responses(&ctx, &owner, &prompt.id, ResponseFilter::RESPONSE_FILTER_PUBLISHED).await.len(), 1);
    assert_eq!(
        ctx.call::<_, SetResponsePublishedResponse>(
            PROMPTS,
            "SetResponsePublished",
            &second,
            &publish(&plain.id, true)
        )
        .await
        .unwrap_err()
        .reason,
        "RESPONSE_NOT_FOUND"
    );
    // What the filter caught has to be released before it can go on the page.
    assert_eq!(
        ctx.call::<_, SetResponsePublishedResponse>(PROMPTS, "SetResponsePublished", &owner, &publish(&held.id, true))
            .await
            .unwrap_err()
            .reason,
        "RESPONSE_NOT_FOUND"
    );
    let unpublished: SetResponsePublishedResponse =
        ctx.call(PROMPTS, "SetResponsePublished", &owner, &publish(&kind.id, false)).await.unwrap();
    assert!(!unpublished.response.as_option().unwrap().published);
    assert!(page_of(&ctx, &second, &prompt.id).await.unwrap().responses.is_empty());

    // Releasing and deleting.
    let release = ReleaseResponseRequest { response_id: held.id.clone(), ..Default::default() };
    assert_eq!(
        ctx.call::<_, ReleaseResponseResponse>(PROMPTS, "ReleaseResponse", &third, &release).await.unwrap_err().reason,
        "RESPONSE_NOT_FOUND"
    );
    let released: ReleaseResponseResponse = ctx.call(PROMPTS, "ReleaseResponse", &owner, &release).await.unwrap();
    assert!(!released.response.as_option().unwrap().held);
    assert_eq!(responses(&ctx, &owner, &prompt.id, ResponseFilter::RESPONSE_FILTER_RECEIVED).await.len(), 3);
    let delete = DeleteResponseRequest { response_id: plain.id.clone(), ..Default::default() };
    assert_eq!(
        ctx.call::<_, DeleteResponseResponse>(PROMPTS, "DeleteResponse", &second, &delete).await.unwrap_err().reason,
        "RESPONSE_NOT_FOUND"
    );
    let _: DeleteResponseResponse = ctx.call(PROMPTS, "DeleteResponse", &owner, &delete).await.unwrap();
    assert_eq!(responses(&ctx, &owner, &prompt.id, ResponseFilter::RESPONSE_FILTER_RECEIVED).await.len(), 2);
    assert_eq!(
        ctx.call::<_, DeleteResponseResponse>(PROMPTS, "DeleteResponse", &owner, &delete).await.unwrap_err().reason,
        "RESPONSE_NOT_FOUND"
    );
    // Its writer still sees it as sent, and cannot respond again.
    let writer = page_of(&ctx, &second, &prompt.id).await.unwrap();
    assert!(writer.prompt.as_option().unwrap().responded && writer.my_response.is_set());
    assert_eq!(respond(&ctx, &second, &prompt.id, "もう一回", false).await.unwrap_err().reason, "ALREADY_RESPONDED");

    // Only the owner lists what came in.
    let listing = ListPromptResponsesRequest { prompt_id: prompt.id.clone(), ..Default::default() };
    assert_eq!(
        ctx.call::<_, ListPromptResponsesResponse>(PROMPTS, "ListPromptResponses", &first, &listing)
            .await
            .unwrap_err()
            .reason,
        "PROMPT_NOT_FOUND"
    );
    let unknown = ListPromptResponsesRequest { filter: EnumValue::from(9), ..listing };
    assert_eq!(
        ctx.call::<_, ListPromptResponsesResponse>(PROMPTS, "ListPromptResponses", &owner, &unknown)
            .await
            .unwrap_err()
            .reason,
        "RESPONSE_FILTER_INVALID"
    );
}

#[tokio::test]
async fn a_blocked_responder_is_told_exactly_what_anyone_is_told() {
    let ctx = start().await;
    let (owner, blocked, other) = (ctx.user("Owner").await, ctx.user(RESPONDER).await, ctx.user("Other").await);
    ctx.befriend(&owner, &blocked).await;
    ctx.befriend(&owner, &other).await;
    let first = post(&ctx, &owner, "一つ目のお題").await;
    let unkind = respond(&ctx, &blocked, &first.id, "いやな回答", false).await.unwrap();
    let request = BlockSenderRequest {
        target: Some(block_sender_request::Target::PromptResponseId(unkind.id.clone())),
        ..Default::default()
    };
    let block: BlockSenderResponse = ctx.call(BOX, "BlockSender", &owner, &request).await.unwrap();
    let block = block.block.as_option().unwrap();
    assert_eq!(
        (block.source.as_known(), block.preview.as_str()),
        (Some(BlockSource::BLOCK_SOURCE_PROMPT_RESPONSE), "いやな回答")
    );
    assert_no_trace_of(&blocked, RESPONDER, block, "the block");
    assert_eq!(
        responses(&ctx, &owner, &first.id, ResponseFilter::RESPONSE_FILTER_RECEIVED).await.len(),
        1,
        "what is already there stays"
    );

    let second = post(&ctx, &owner, "二つ目のお題").await;
    let mut pushes = ctx.app.listen_user(owner.id).await;
    ctx.new_events().await;
    let respond_to_second =
        RespondToPromptRequest { prompt_id: second.id.clone(), text: "おなじ回答".into(), ..Default::default() };
    let dropped = ctx.json(PROMPTS, "RespondToPrompt", &blocked, &respond_to_second).await;
    let dropped_events = ctx.new_events().await;
    let arrived = ctx.json(PROMPTS, "RespondToPrompt", &other, &respond_to_second).await;
    let arrived_events = ctx.new_events().await;
    let told = |mut response: serde_json::Value| {
        for key in ["id", "createdAt"] {
            response.as_object_mut().unwrap().remove(key);
        }
        response
    };
    assert_eq!(
        told(dropped["response"].clone()),
        told(arrived["response"].clone()),
        "the response does not depend on the block"
    );
    assert_eq!(shape(&dropped_events), shape(&arrived_events), "the request writes the same events either way");
    assert_eq!((dropped_events[0].topic.as_str(), dropped_events[0].actor_id), ("ask.response.stored", None));
    ctx.handle(&dropped_events).await;
    ctx.handle(&arrived_events).await;

    // Each sees their own response as sent, and neither can respond again.
    for who in [&blocked, &other] {
        let page = page_of(&ctx, who, &second.id).await.unwrap();
        assert!(
            page.prompt.as_option().unwrap().responded
                && page.my_response.as_option().is_some_and(|mine| mine.text == "おなじ回答"),
            "{}",
            who.handle
        );
        assert_eq!(
            respond(&ctx, who, &second.id, "もう一回", false).await.unwrap_err().reason,
            "ALREADY_RESPONDED",
            "{}",
            who.handle
        );
    }
    // The owner got one of the two.
    let received = responses(&ctx, &owner, &second.id, ResponseFilter::RESPONSE_FILTER_RECEIVED).await;
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].id, arrived["response"]["id"].as_str().unwrap());
    assert_eq!(page_of(&ctx, &owner, &second.id).await.unwrap().prompt.as_option().unwrap().response_count, 1);
    assert!(matches!(next_push(&mut pushes).await, Some(Push::PromptResponseReceived(_))));
    assert!(stays_quiet(&mut pushes).await, "nothing is pushed for the dropped response");
    let dropped_id = dropped["response"]["id"].as_str().unwrap().to_owned();
    let publish = SetResponsePublishedRequest { response_id: dropped_id, published: true, ..Default::default() };
    assert_eq!(
        ctx.call::<_, SetResponsePublishedResponse>(PROMPTS, "SetResponsePublished", &owner, &publish)
            .await
            .unwrap_err()
            .reason,
        "RESPONSE_NOT_FOUND"
    );
}

#[tokio::test]
async fn prompts_are_listed_for_their_owner_and_their_friends() {
    let ctx = start().await;
    let (owner, friend, muted, stranger) =
        (ctx.user("Owner").await, ctx.user("Friend").await, ctx.user("Muted").await, ctx.user("Stranger").await);
    ctx.befriend(&friend, &owner).await;
    ctx.befriend(&friend, &muted).await;
    let open = post(&ctx, &owner, "ひらいてるお題").await;
    let closed = post(&ctx, &owner, "しめたお題").await;
    let _: ClosePromptResponse = ctx
        .call(
            PROMPTS,
            "ClosePrompt",
            &owner,
            &ClosePromptRequest { prompt_id: closed.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    post(&ctx, &muted, "ミュートした友達のお題").await;
    post(&ctx, &stranger, "知らない人のお題").await;
    ctx.mute(&friend, &muted).await;

    let list = |viewer: &TestUser, user: String| {
        let (ctx, viewer) = (&ctx, viewer.clone());
        async move {
            ctx.call::<_, ListPromptsResponse>(
                PROMPTS,
                "ListPrompts",
                &viewer,
                &ListPromptsRequest { user, ..Default::default() },
            )
            .await
        }
    };
    let ids = |response: ListPromptsResponse| response.prompts.into_iter().map(|p| p.id).collect::<Vec<_>>();
    assert_eq!(
        ids(list(&owner, String::new()).await.unwrap()),
        vec![closed.id.clone(), open.id.clone()],
        "one's own, newest first"
    );
    let for_friend = list(&friend, owner.handle.clone()).await.unwrap();
    assert!(for_friend.prompts.iter().any(|p| p.id == open.id && p.can_respond));
    assert_eq!(ids(for_friend), vec![closed.id.clone(), open.id.clone()]);
    assert!(list(&stranger, owner.id.to_string()).await.unwrap().prompts.is_empty(), "prompts are for friends");
    assert_eq!(list(&friend, "no_such_user_here".into()).await.unwrap_err().reason, "USER_NOT_FOUND");

    // The feed of friends' open prompts.
    let feed: ListFriendPromptsResponse =
        ctx.call(PROMPTS, "ListFriendPrompts", &friend, &ListFriendPromptsRequest::default()).await.unwrap();
    assert_eq!(
        feed.prompts.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec![open.id.as_str()],
        "open, from a friend who is not muted"
    );
    let feed: ListFriendPromptsResponse =
        ctx.call(PROMPTS, "ListFriendPrompts", &stranger, &ListFriendPromptsRequest::default()).await.unwrap();
    assert!(feed.prompts.is_empty());
}

#[tokio::test]
async fn moderators_see_who_wrote_a_response_and_reports_reach_them() {
    let ctx = start().await;
    let (owner, responder, friend) = (ctx.user("Owner").await, ctx.user(RESPONDER).await, ctx.user("Friend").await);
    let moderator = ctx.app.user_with_roles("Moderator", &["moderator"]).await;
    ctx.befriend(&owner, &responder).await;
    ctx.befriend(&owner, &friend).await;
    let prompt = post(&ctx, &owner, "私の第一印象は?").await;
    let response = respond(&ctx, &responder, &prompt.id, "ひどいことを書いた回答", false).await.unwrap();

    let request = InspectPromptResponseRequest { response_id: response.id.clone(), ..Default::default() };
    let seen: InspectPromptResponseResponse =
        ctx.call(MODERATION, "InspectPromptResponse", &moderator, &request).await.unwrap();
    assert_eq!(seen.record.as_option().unwrap().sender.as_option().unwrap().id, responder.id.to_string());
    assert_eq!(
        (seen.prompt_id.as_str(), seen.prompt_text.as_str(), seen.text.as_str()),
        (prompt.id.as_str(), "私の第一印象は?", "ひどいことを書いた回答")
    );
    let refused = ctx
        .call::<_, InspectPromptResponseResponse>(MODERATION, "InspectPromptResponse", &owner, &request)
        .await
        .unwrap_err();
    assert_eq!(refused.reason, "FORBIDDEN");
    let missing = InspectPromptResponseRequest { response_id: Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(
        ctx.call::<_, InspectPromptResponseResponse>(MODERATION, "InspectPromptResponse", &moderator, &missing)
            .await
            .unwrap_err()
            .reason,
        "RESPONSE_NOT_FOUND"
    );

    // Prompts and responses are reported as questions.
    let subjects = &ctx.app.core.subjects;
    let kind = SubjectType::SUBJECT_TYPE_QUESTION;
    let for_owner = subjects.resolve(kind, &response.id, Some(owner.id)).await.unwrap();
    assert!(for_owner.owner_id.is_nil() && !for_owner.interactions_enabled);
    assert_eq!(subjects.resolve(kind, &response.id, Some(moderator.id)).await.unwrap().owner_id, responder.id);
    assert_eq!(
        subjects.resolve(kind, &response.id, Some(friend.id)).await.unwrap_err().reason,
        "SUBJECT_NOT_FOUND",
        "not on the page yet"
    );
    let the_prompt = subjects.resolve(kind, &prompt.id, Some(friend.id)).await.unwrap();
    assert_eq!((the_prompt.owner_id, the_prompt.interactions_enabled), (owner.id, false));

    // A takedown removes the response for everyone, its writer included.
    ctx.announce("admin.content.removed", serde_json::json!({ "subject_type": kind as i32, "subject_id": response.id, "reason": "harassment", "moderator_id": moderator.id }))
        .await;
    assert!(responses(&ctx, &owner, &prompt.id, ResponseFilter::RESPONSE_FILTER_RECEIVED).await.is_empty());
    assert!(page_of(&ctx, &responder, &prompt.id).await.unwrap().my_response.is_unset());
    // And a prompt goes with everything on it.
    ctx.announce("admin.content.removed", serde_json::json!({ "subject_type": kind as i32, "subject_id": prompt.id, "reason": "bullying", "moderator_id": moderator.id })).await;
    assert_eq!(page_of(&ctx, &owner, &prompt.id).await.unwrap_err().reason, "PROMPT_NOT_FOUND");
}
