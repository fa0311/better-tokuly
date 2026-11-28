//! Tokuly Bot, against a local stand-in for the Anthropic Messages API
//! (`TOKULY_ANTHROPIC_BASE_URL`). The stub records what the server sends,
//! so the tests can check that only the messages of people who allow it
//! leave the server, labelled without names. Nothing here talks to a real
//! API. No job runner runs in tests: what job `chat.bot_reply` would do is
//! done by calling [`tokuly_chat::Chat::answer_as_bot`].
mod common;

use std::sync::{Arc, Mutex};

use axum::{Json, Router, extract::State, routing::post};
use common::*;
use tokuly_chat::ChatConfig;
use tokuly_proto::chat::*;
use tokuly_testkit::TestUser;
use uuid::Uuid;

const ANSWER: &str = "やっほー！なんでも聞いてね✨";

#[derive(Clone, Default)]
struct Stub {
    prompts: Arc<Mutex<Vec<serde_json::Value>>>,
}

impl Stub {
    fn count(&self) -> usize {
        self.prompts.lock().unwrap().len()
    }

    /// The user turn of the last request: the conversation and the question.
    fn last_prompt(&self) -> String {
        let body = self.prompts.lock().unwrap().last().cloned().expect("the stub received a request");
        body["messages"][0]["content"].as_str().unwrap_or_default().to_owned()
    }

    fn last_body(&self) -> serde_json::Value {
        self.prompts.lock().unwrap().last().cloned().expect("the stub received a request")
    }
}

/// Answers like the Messages API would, steered by marker words in the question.
async fn messages(State(stub): State<Stub>, Json(body): Json<serde_json::Value>) -> Json<serde_json::Value> {
    stub.prompts.lock().unwrap().push(body.clone());
    let prompt = body["messages"][0]["content"].as_str().unwrap_or_default().to_owned();
    let question = prompt.split("<question>").nth(1).unwrap_or_default();
    if question.contains("REFUSE") {
        return Json(
            serde_json::json!({ "type": "message", "role": "assistant", "content": [], "stop_reason": "refusal" }),
        );
    }
    let answer = serde_json::json!({ "reply": ANSWER });
    Json(serde_json::json!({
        "type": "message",
        "role": "assistant",
        "content": [{ "type": "text", "text": answer.to_string() }],
        "stop_reason": "end_turn",
    }))
}

async fn start_with_stub(change: impl FnOnce(&mut ChatConfig)) -> (Ctx, Stub) {
    let stub = Stub::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
    let address = listener.local_addr().unwrap();
    let router = Router::new().route("/v1/messages", post(messages)).with_state(stub.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.expect("stub server") });
    let mut cfg = ChatConfig {
        anthropic_api_key: "sk-ant-test-key".into(),
        anthropic_base_url: format!("http://{address}/"),
        ai_timeout_seconds: 15,
        ai_allow_loopback: true,
        ..ChatConfig::default()
    };
    change(&mut cfg);
    (start_with(cfg).await, stub)
}

async fn settings_of(ctx: &Ctx, user: &TestUser) -> AssistSettings {
    let response: GetAssistSettingsResponse =
        ctx.call("Assist", "GetAssistSettings", user, &GetAssistSettingsRequest::default()).await.unwrap();
    response.settings.into_option().expect("settings")
}

async fn open_bot(ctx: &Ctx, user: &TestUser) -> Conversation {
    let response: OpenBotConversationResponse =
        ctx.call("Assist", "OpenBotConversation", user, &OpenBotConversationRequest::default()).await.unwrap();
    response.conversation.into_option().expect("conversation")
}

async fn set_bot_enabled(ctx: &Ctx, user: &TestUser, enabled: bool) -> AssistSettings {
    let response: SetBotEnabledResponse = ctx
        .call("Assist", "SetBotEnabled", user, &SetBotEnabledRequest { enabled, ..Default::default() })
        .await
        .unwrap();
    response.settings.into_option().unwrap()
}

async fn make_minor(ctx: &Ctx, user: &TestUser) {
    let today = chrono::Utc::now().date_naive();
    sqlx::query("UPDATE users SET birthdate = $2::date WHERE id = $1")
        .bind(user.id)
        .bind(format!("{}-01-01", chrono::Datelike::year(&today) - 15))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
}

async fn bot_id(ctx: &Ctx, user: &TestUser) -> Uuid {
    settings_of(ctx, user).await.bot.as_option().expect("the bot is available").id.parse().unwrap()
}

/// Sends a text and has the bot (maybe) answer it, as the job would.
async fn ask(ctx: &Ctx, sender: &TestUser, conversation_id: &str, text: &str) -> Message {
    let message = ctx.text(sender, conversation_id, text).await;
    ctx.chat.answer_as_bot(message.id.parse().unwrap()).await.unwrap();
    message
}

/// The bot's answers in a conversation, as `viewer` sees them.
async fn answers(ctx: &Ctx, viewer: &TestUser, conversation_id: &str, bot: Uuid) -> Vec<Message> {
    let bot = bot.to_string();
    ctx.latest(viewer, conversation_id)
        .await
        .into_iter()
        .filter(|message| message.sender.as_option().is_some_and(|sender| sender.id == bot))
        .collect()
}

async fn add_members(
    ctx: &Ctx,
    user: &TestUser,
    conversation_id: &str,
    ids: &[Uuid],
) -> Result<AddMembersResponse, String> {
    let request = AddMembersRequest {
        conversation_id: conversation_id.to_owned(),
        user_ids: ids.iter().map(Uuid::to_string).collect(),
        ..Default::default()
    };
    ctx.call("Conversation", "AddMembers", user, &request).await.map_err(|error| error.reason)
}

#[tokio::test]
async fn without_an_ai_provider_there_is_no_bot() {
    let ctx = start().await;
    let taro = ctx.app.user("taro").await;
    assert!(settings_of(&ctx, &taro).await.bot.as_option().is_none());
    assert_eq!(
        ctx.reason("Assist", "OpenBotConversation", &taro, &OpenBotConversationRequest::default()).await,
        "AI_DISABLED"
    );
    assert_eq!(
        ctx.anonymous_reason("Assist", "OpenBotConversation", &OpenBotConversationRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );

    // The operator switched the bot off while the assistant stays.
    let (ctx, stub) = start_with_stub(|cfg| cfg.bot_enabled = false).await;
    let taro = ctx.app.user("taro").await;
    assert!(settings_of(&ctx, &taro).await.bot.as_option().is_none());
    assert_eq!(
        ctx.reason("Assist", "OpenBotConversation", &taro, &OpenBotConversationRequest::default()).await,
        "AI_DISABLED"
    );
    assert_eq!(stub.count(), 0);
}

#[tokio::test]
async fn the_bot_answers_in_its_direct_conversation() {
    let (ctx, stub) = start_with_stub(|_| {}).await;
    let taro = ctx.app.user("taro").await;
    let settings = settings_of(&ctx, &taro).await;
    let bot = settings.bot.as_option().expect("the bot").clone();
    assert!(bot.bot, "shown with the bot badge");
    assert_eq!((bot.handle.as_str(), bot.display_name.as_str()), ("tokuly_bot", "Tokuly Bot"));
    assert!(settings.bot_enabled && settings.bot_enabled_is_default, "on for an adult");

    let direct = open_bot(&ctx, &taro).await;
    assert_eq!(open_bot(&ctx, &taro).await.id, direct.id, "one conversation");
    assert_eq!(ctx.direct(&taro, &TestUser { id: bot.id.parse().unwrap(), ..taro.clone() }).await.id, direct.id);
    assert!(direct.members.iter().any(|member| member.id == bot.id && member.bot));

    ctx.text(&taro, &direct.id, "こんにちは").await;
    let question = ask(&ctx, &taro, &direct.id, "おすすめの映画ある？").await;
    let replies = answers(&ctx, &taro, &direct.id, bot.id.parse().unwrap()).await;
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].text, ANSWER);
    assert_eq!(replies[0].reply_to.as_option().map(|reply| reply.message_id.as_str()), Some(question.id.as_str()));

    // What went to the AI service: the conversation and the question, without names.
    assert_eq!(stub.count(), 1);
    let prompt = stub.last_prompt();
    assert!(prompt.contains("asker: こんにちは"), "{prompt}");
    assert!(prompt.ends_with("<question>\nおすすめの映画ある？\n</question>"), "{prompt}");
    let body = stub.last_body().to_string();
    for secret in [taro.id.to_string(), taro.handle.clone(), "taro".to_owned()] {
        assert!(!body.contains(&secret), "{secret} was sent to the AI service");
    }
    assert!(stub.last_body()["system"].as_str().unwrap().contains("Tokuly Bot"));

    // A job delivered again answers nothing twice, nor do the bot's own messages get an answer.
    ctx.chat.answer_as_bot(question.id.parse().unwrap()).await.unwrap();
    ctx.chat.answer_as_bot(replies[0].id.parse().unwrap()).await.unwrap();
    assert_eq!(stub.count(), 1);
    assert_eq!(answers(&ctx, &taro, &direct.id, bot.id.parse().unwrap()).await.len(), 1);

    // Its earlier answers are part of the conversation it sees.
    ask(&ctx, &taro, &direct.id, "ほかには？").await;
    assert!(stub.last_prompt().contains(&format!("tokuly_bot: {ANSWER}")));
}

#[tokio::test]
async fn the_same_question_delivered_at_once_is_answered_once() {
    let (ctx, stub) = start_with_stub(|_| {}).await;
    let taro = ctx.app.user("taro").await;
    let direct = open_bot(&ctx, &taro).await;
    let bot = bot_id(&ctx, &taro).await;
    let question = ctx.text(&taro, &direct.id, "いっせいに").await;
    let id: Uuid = question.id.parse().unwrap();
    let results = futures::future::join_all((0..6).map(|_| ctx.chat.answer_as_bot(id))).await;
    assert!(results.iter().all(Result::is_ok));
    assert_eq!(stub.count(), 1);
    assert_eq!(answers(&ctx, &taro, &direct.id, bot).await.len(), 1);
}

#[tokio::test]
async fn opening_the_bot_conversation_at_once_from_one_account_makes_one() {
    let (ctx, _stub) = start_with_stub(|_| {}).await;
    let taro = ctx.app.user("taro").await;
    let opened = futures::future::join_all((0..8).map(|_| open_bot(&ctx, &taro))).await;
    assert!(opened.iter().all(|conversation| conversation.id == opened[0].id));
}

#[tokio::test]
async fn in_a_group_the_bot_answers_mentions_with_the_messages_of_people_who_allow_it() {
    let (ctx, stub) = start_with_stub(|_| {}).await;
    let (group, owner, members) = ctx.group_of("遊びの相談", 3).await;
    let (adult, minor, blocked) = (&members[0], &members[1], &members[2]);
    make_minor(&ctx, minor).await;
    let bot = bot_id(&ctx, &owner).await;

    // Anyone whose bot is on may bring it in, without being its friend.
    add_members(&ctx, &owner, &group.id, &[bot]).await.unwrap();
    let listed: ListMembersResponse = ctx
        .call(
            "Conversation",
            "ListMembers",
            &owner,
            &ListMembersRequest { conversation_id: group.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(listed.members.iter().any(|member| member.user.as_option().is_some_and(|user| user.bot)));

    ctx.text(adult, &group.id, "土曜は駅前に10時で！").await;
    ctx.text(minor, &group.id, "わたしの秘密の予定").await;
    ctx.text(blocked, &group.id, "ブロックされた人の話").await;
    ctx.block(&owner, blocked).await;

    // Without a mention, the bot stays out of it.
    ask(&ctx, adult, &group.id, "楽しみだね").await;
    assert_eq!(stub.count(), 0);

    let question = ask(&ctx, &owner, &group.id, "@tokuly_bot 集合何時だっけ？").await;
    assert_eq!(question.mentions.len(), 1);
    assert_eq!(stub.count(), 1);
    let prompt = stub.last_prompt();
    assert!(prompt.contains("member1: 土曜は駅前に10時で！"), "{prompt}");
    assert!(prompt.contains("member1: 楽しみだね"), "{prompt}");
    assert!(!prompt.contains("秘密"), "a minor's messages need their consent: {prompt}");
    assert!(!prompt.contains("ブロック"), "nothing of someone in a block with the asker: {prompt}");
    for user in [&owner, adult, minor, blocked] {
        assert!(!stub.last_body().to_string().contains(&user.handle));
    }
    let replies = answers(&ctx, adult, &group.id, bot).await;
    assert_eq!(replies.len(), 1, "every member sees the answer");

    // A reply to the bot's answer addresses it too.
    let request = SendMessageRequest {
        conversation_id: group.id.clone(),
        client_id: client_id(),
        text: "ありがとう！".into(),
        reply_to_message_id: replies[0].id.clone(),
        ..Default::default()
    };
    let thanks = ctx.send(adult, &request).await.unwrap().message.into_option().unwrap();
    ctx.chat.answer_as_bot(thanks.id.parse().unwrap()).await.unwrap();
    assert_eq!(stub.count(), 2);
    assert!(stub.last_prompt().contains("asker: 土曜は駅前に10時で！"), "labels follow the asker");

    // The minor, whose bot is off by default, gets no answer and cannot bring it anywhere.
    ask(&ctx, minor, &group.id, "@tokuly_bot ねえねえ").await;
    assert_eq!(stub.count(), 2);

    // Once they switch it on it answers them, but their consent is still
    // off: neither their question nor the answer to it goes into anyone
    // else's request.
    set_bot_enabled(&ctx, minor, true).await;
    ask(&ctx, minor, &group.id, "@tokuly_bot わたしの秘密を覚えてて").await;
    assert_eq!(stub.count(), 3);
    assert_eq!(answers(&ctx, &owner, &group.id, bot).await.len(), 3);
    ask(&ctx, &owner, &group.id, "@tokuly_bot もう一回教えて").await;
    let prompt = stub.last_prompt();
    assert!(!prompt.contains("秘密"), "{prompt}");
    assert_eq!(prompt.matches("\ntokuly_bot: ").count(), 2, "the answers to the owner and the adult only: {prompt}");
}

#[tokio::test]
async fn the_bot_is_off_for_minors_until_they_switch_it_on() {
    let (ctx, stub) = start_with_stub(|_| {}).await;
    let (teen, friend) = ctx.friends("teen", "friend").await;
    make_minor(&ctx, &teen).await;
    let settings = settings_of(&ctx, &teen).await;
    assert!(!settings.bot_enabled && settings.bot_enabled_is_default);
    assert_eq!(
        ctx.reason("Assist", "OpenBotConversation", &teen, &OpenBotConversationRequest::default()).await,
        "BOT_TURNED_OFF"
    );
    let group = ctx.group(&teen, "クラス", &[&friend]).await;
    let bot = bot_id(&ctx, &teen).await;
    assert_eq!(add_members(&ctx, &teen, &group.id, &[bot]).await.unwrap_err(), "BOT_TURNED_OFF");

    // Their own choice, which a later birthdate event does not undo.
    let chosen = set_bot_enabled(&ctx, &teen, true).await;
    assert!(chosen.bot_enabled && !chosen.bot_enabled_is_default);
    let given = tokuly_core::events::Event {
        id: Uuid::now_v7(),
        topic: "identity.user.birthdate_given".into(),
        actor_id: Some(teen.id),
        occurred_at: chrono::Utc::now(),
        data: serde_json::json!({ "user_id": teen.id, "minor": true }),
    };
    ctx.chat.handle_event(&given).await.unwrap();
    assert!(settings_of(&ctx, &teen).await.bot_enabled);
    let direct = open_bot(&ctx, &teen).await;
    ask(&ctx, &teen, &direct.id, "宿題てつだって").await;
    assert_eq!(stub.count(), 1);

    // A minor who never chose gets the minors' default from a late birthdate.
    let late = ctx.app.user("late").await;
    assert!(settings_of(&ctx, &late).await.bot_enabled);
    ctx.chat
        .handle_event(&tokuly_core::events::Event {
            data: serde_json::json!({ "user_id": late.id, "minor": true }),
            ..given.clone()
        })
        .await
        .unwrap();
    assert!(!settings_of(&ctx, &late).await.bot_enabled);

    // Switched off: the bot stays silent in the conversation that is already there.
    set_bot_enabled(&ctx, &teen, false).await;
    ask(&ctx, &teen, &direct.id, "まだいる？").await;
    assert_eq!(stub.count(), 1);
    assert_eq!(answers(&ctx, &teen, &direct.id, bot).await.len(), 1);
}

#[tokio::test]
async fn limits_refusals_and_a_switched_off_assistant_get_a_fixed_answer() {
    let (ctx, stub) = start_with_stub(|cfg| cfg.bot_hourly_limit = 2).await;
    let taro = ctx.app.user("taro").await;
    let direct = open_bot(&ctx, &taro).await;
    let bot = bot_id(&ctx, &taro).await;

    ask(&ctx, &taro, &direct.id, "REFUSE this").await;
    ask(&ctx, &taro, &direct.id, "ふつうの質問").await;
    // The limit is reached: said once, then silence.
    ask(&ctx, &taro, &direct.id, "もう一個").await;
    ask(&ctx, &taro, &direct.id, "さらに").await;
    assert_eq!(stub.count(), 2);
    let texts: Vec<String> = answers(&ctx, &taro, &direct.id, bot).await.into_iter().map(|m| m.text).collect();
    assert_eq!(texts.len(), 3, "{texts:?}");
    assert!(texts[0].contains("お答えできない"), "{texts:?}");
    assert_eq!(texts[1], ANSWER);
    assert!(texts[2].contains("休憩"), "{texts:?}");

    // The asker switched the AI assistant off: the bot says where to turn it on.
    let hanako = ctx.app.user("hanako").await;
    let direct = open_bot(&ctx, &hanako).await;
    let _: ChooseAssistProviderResponse = ctx
        .call(
            "Assist",
            "ChooseAssistProvider",
            &hanako,
            &ChooseAssistProviderRequest { off: true, ..Default::default() },
        )
        .await
        .unwrap();
    ask(&ctx, &hanako, &direct.id, "こんにちは").await;
    assert_eq!(stub.count(), 2);
    let texts: Vec<String> = answers(&ctx, &hanako, &direct.id, bot).await.into_iter().map(|m| m.text).collect();
    assert_eq!(texts.len(), 1);
    assert!(texts[0].contains("AI 設定"), "{texts:?}");
    assert_eq!(
        ctx.reason("Assist", "OpenBotConversation", &hanako, &OpenBotConversationRequest::default()).await,
        "AI_TURNED_OFF"
    );

    // In a group the others would learn about her settings from that text: silence there.
    let friend = ctx.app.user("friend").await;
    ctx.befriend(&friend, &hanako).await;
    let group = ctx.group(&friend, "放課後", &[&hanako]).await;
    add_members(&ctx, &friend, &group.id, &[bot]).await.unwrap();
    ask(&ctx, &hanako, &group.id, "@tokuly_bot いる？").await;
    assert_eq!(stub.count(), 2);
    assert!(answers(&ctx, &friend, &group.id, bot).await.is_empty());
}

#[tokio::test]
async fn a_block_silences_the_bot() {
    let (ctx, stub) = start_with_stub(|_| {}).await;
    let taro = ctx.app.user("taro").await;
    let direct = open_bot(&ctx, &taro).await;
    let bot = bot_id(&ctx, &taro).await;
    ctx.block(&taro, &TestUser { id: bot, ..taro.clone() }).await;
    let error = ctx
        .send(
            &taro,
            &SendMessageRequest {
                conversation_id: direct.id.clone(),
                client_id: client_id(),
                text: "おーい".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.reason, "CONVERSATION_READ_ONLY");
    assert_eq!(stub.count(), 0);

    // Someone who blocked the bot before ever talking to it is told so,
    // and cannot bring it into a group either.
    let (hanako, friend) = ctx.friends("hanako", "friend").await;
    ctx.block(&hanako, &TestUser { id: bot, ..hanako.clone() }).await;
    assert_eq!(
        ctx.reason("Assist", "OpenBotConversation", &hanako, &OpenBotConversationRequest::default()).await,
        "BOT_BLOCKED"
    );
    assert_eq!(
        ctx.reason(
            "Conversation",
            "OpenDirect",
            &hanako,
            &OpenDirectRequest { user_id: bot.to_string(), ..Default::default() }
        )
        .await,
        "BOT_BLOCKED"
    );
    let group = ctx.group(&hanako, "クラス", &[&friend]).await;
    assert_eq!(add_members(&ctx, &hanako, &group.id, &[bot]).await.unwrap_err(), "BOT_BLOCKED");
}
