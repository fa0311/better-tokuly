//! `AssistService`, against a local stand-in for the Anthropic Messages
//! API and OpenAI's Chat Completions API (`TOKULY_ANTHROPIC_BASE_URL`,
//! `TOKULY_OPENAI_BASE_URL`). The stub records what the server sends, so the
//! tests can check that only the text a feature needs leaves the server, and
//! only with the consent of the people quoted. Nothing here talks to a real
//! API. The operator settings (staff console) are tested in
//! `assist_admin.rs`: they are global, and these tests run on the
//! environment's values.
mod common;

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use axum::{Json, Router, extract::State, http::HeaderMap, http::StatusCode, routing::post};
use common::*;
use tokuly_chat::ChatConfig;
use tokuly_proto::chat::*;
use tokuly_testkit::TestUser;
use uuid::Uuid;

const API_KEY: &str = "sk-ant-test-key";

/// One request the stub received.
#[derive(Clone)]
struct Recorded {
    headers: HashMap<String, String>,
    body: serde_json::Value,
}

impl Recorded {
    /// The text of the user turn: what was sent about the user's content.
    fn prompt(&self) -> &str {
        self.body["messages"]
            .as_array()
            .and_then(|turns| turns.last())
            .and_then(|turn| turn["content"].as_str())
            .unwrap_or_default()
    }

    /// The system prompt, in either protocol.
    fn system(&self) -> &str {
        self.body["system"].as_str().or_else(|| self.body["messages"][0]["content"].as_str()).unwrap_or_default()
    }

    /// Which API was called.
    fn openai(&self) -> bool {
        self.body.get("system").is_none()
    }
}

#[derive(Clone, Default)]
pub struct Stub {
    requests: Arc<Mutex<Vec<Recorded>>>,
    /// Answer `response_format` with 400, like a server without schema support.
    rejects_schema: Arc<AtomicBool>,
}

impl Stub {
    fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }

    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    fn last(&self) -> Recorded {
        self.requests().pop().expect("the stub received a request")
    }
}

fn error(status: StatusCode, kind: &str) -> (StatusCode, Json<serde_json::Value>) {
    (status, Json(serde_json::json!({ "type": "error", "error": { "type": kind, "message": "stub" } })))
}

/// The answer for a request, by the system prompt.
fn answer_for(system: &str) -> serde_json::Value {
    if system.contains("rewrite") {
        serde_json::json!({ "text": "おはよ〜！今日もがんばろっ✨" })
    } else if system.contains("quick replies") {
        serde_json::json!({ "replies": ["いいね！行こう", "何時から？", "ごめん、その日は無理かも", "四つ目は多すぎ"] })
    } else {
        serde_json::json!({ "translation": "Good morning!", "source_language": "JA" })
    }
}

fn record(stub: &Stub, headers: &HeaderMap, body: serde_json::Value) -> (Recorded, usize) {
    let headers = headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), value.to_str().unwrap_or_default().to_owned()))
        .collect();
    let recorded = Recorded { headers, body };
    let prompt = recorded.prompt().to_owned();
    let mut requests = stub.requests.lock().unwrap();
    requests.push(recorded.clone());
    let attempts = requests.iter().filter(|request| request.prompt() == prompt).count();
    (recorded, attempts)
}

/// Answers like OpenAI's Chat Completions API would (or a compatible server
/// without schema support, when told to).
async fn completions(
    State(stub): State<Stub>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let (recorded, _) = record(&stub, &headers, body);
    let schema = recorded.body.get("response_format").is_some();
    if schema && stub.rejects_schema.load(Ordering::SeqCst) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "'response_format.type' must be 'json_object' or 'text'" })),
        );
    }
    if recorded.prompt().contains("BADKEY") {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": { "type": "invalid_request_error", "message": "stub" } })),
        );
    }
    if recorded.prompt().contains("REFUSE") {
        return (
            StatusCode::OK,
            Json(
                serde_json::json!({ "choices": [{ "index": 0, "message": { "role": "assistant", "content": null, "refusal": "I can't help with that." }, "finish_reason": "stop" }] }),
            ),
        );
    }
    let answer = answer_for(recorded.system());
    // Without a schema, a local model wraps its JSON in prose and fences.
    let content = if schema { answer.to_string() } else { format!("Sure! Here it is:\n```json\n{answer}\n```") };
    let response = serde_json::json!({
        "id": "chatcmpl-stub",
        "object": "chat.completion",
        "model": recorded.body["model"],
        "choices": [{ "index": 0, "message": { "role": "assistant", "content": content, "refusal": null }, "finish_reason": "stop" }],
        "usage": { "prompt_tokens": 100, "completion_tokens": 20 },
    });
    (StatusCode::OK, Json(response))
}

/// Answers like the Messages API would, steered by marker words in the prompt.
async fn messages(
    State(stub): State<Stub>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let (recorded, attempts) = record(&stub, &headers, body);
    let prompt = recorded.prompt().to_owned();
    let system = recorded.system().to_owned();
    if prompt.contains("BADKEY") {
        return error(StatusCode::UNAUTHORIZED, "authentication_error");
    }
    if prompt.contains("OVERLOADED") || (prompt.contains("FLAKY") && attempts < 3) {
        return error(StatusCode::from_u16(529).unwrap(), "overloaded_error");
    }
    if prompt.contains("REFUSE") {
        return (
            StatusCode::OK,
            Json(
                serde_json::json!({ "type": "message", "role": "assistant", "content": [], "stop_reason": "refusal", "stop_details": { "type": "refusal", "category": null } }),
            ),
        );
    }
    let answer = if prompt.contains("LONGANSWER") {
        // A model that runs on and on.
        serde_json::json!({ "translation": "あ".repeat(6000), "source_language": "ja" })
    } else {
        answer_for(&system)
    };
    let response = serde_json::json!({
        "id": "msg_stub",
        "type": "message",
        "role": "assistant",
        "model": "claude-opus-5-5",
        "content": [
            { "type": "thinking", "thinking": "", "signature": "stub" },
            { "type": "text", "text": answer.to_string() },
        ],
        "stop_reason": "end_turn",
        "usage": { "input_tokens": 100, "output_tokens": 20 },
    });
    (StatusCode::OK, Json(response))
}

/// Starts the stub: both APIs on one address.
async fn start_stub() -> (Stub, std::net::SocketAddr) {
    let stub = Stub::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
    let address = listener.local_addr().unwrap();
    let router = Router::new()
        .route("/v1/messages", post(messages))
        .route("/v1/chat/completions", post(completions))
        .with_state(stub.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.expect("stub server") });
    (stub, address)
}

/// Starts the stub and a chat domain whose assistant talks to it: Anthropic
/// on, and OpenAI too when `change` gives it a key.
async fn start_with_stub(change: impl FnOnce(&mut ChatConfig)) -> (Ctx, Stub) {
    let (stub, address) = start_stub().await;
    let mut cfg = ChatConfig {
        anthropic_api_key: API_KEY.into(),
        anthropic_base_url: format!("http://{address}/"),
        openai_base_url: format!("http://{address}/v1"),
        ai_timeout_seconds: 15,
        // The stub is on loopback, on a port of its own.
        ai_allow_loopback: true,
        ..ChatConfig::default()
    };
    change(&mut cfg);
    (start_with(cfg).await, stub)
}

fn rewrite(text: &str, tone: RewriteTone) -> RewriteDraftRequest {
    RewriteDraftRequest { text: text.to_owned(), tone: tone.into(), ..Default::default() }
}

fn translate(message_id: &str, language: &str) -> TranslateMessageRequest {
    TranslateMessageRequest {
        message_id: message_id.to_owned(),
        target_language: language.to_owned(),
        ..Default::default()
    }
}

fn suggest(conversation_id: &str) -> SuggestRepliesRequest {
    SuggestRepliesRequest { conversation_id: conversation_id.to_owned(), ..Default::default() }
}

/// Fails if anything identifying a user is in what was sent.
fn assert_anonymous(sent: &Recorded, users: &[&TestUser]) {
    let body = sent.body.to_string();
    for user in users {
        assert!(!body.contains(&user.id.to_string()), "a user ID was sent to the AI service");
        assert!(!body.contains(&user.handle), "a handle was sent to the AI service");
    }
}

#[tokio::test]
async fn the_assistant_is_off_without_an_api_key() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let direct = ctx.direct(&taro, &hanako).await;
    let message = ctx.text(&hanako, &direct.id, "おはよう").await;

    let error = ctx
        .call::<_, RewriteDraftResponse>(
            "Assist",
            "RewriteDraft",
            &taro,
            &rewrite("おはよう", RewriteTone::REWRITE_TONE_CUTE),
        )
        .await
        .unwrap_err();
    assert_eq!((error.reason.as_str(), error.code.as_str()), ("AI_DISABLED", "unimplemented"));
    assert_eq!(ctx.reason("Assist", "SuggestReplies", &taro, &suggest(&direct.id)).await, "AI_DISABLED");
    assert_eq!(ctx.reason("Assist", "TranslateMessage", &taro, &translate(&message.id, "en")).await, "AI_DISABLED");
    assert_eq!(
        ctx.anonymous_reason("Assist", "RewriteDraft", &rewrite("おはよう", RewriteTone::REWRITE_TONE_CUTE)).await,
        "SIGN_IN_REQUIRED"
    );
}

#[tokio::test]
async fn a_draft_is_rewritten_in_another_tone() {
    let (ctx, stub) = start_with_stub(|_| {}).await;
    let taro = ctx.app.user("taro").await;
    let capabilities: GetCapabilitiesResponse =
        ctx.call("Conversation", "GetCapabilities", &taro, &GetCapabilitiesRequest::default()).await.unwrap();
    assert!(capabilities.capabilities.assist_enabled);

    let rewritten: RewriteDraftResponse = ctx
        .call(
            "Assist",
            "RewriteDraft",
            &taro,
            &rewrite("  おはようございます。本日もよろしくお願いします。 ", RewriteTone::REWRITE_TONE_CUTE),
        )
        .await
        .unwrap();
    assert_eq!(rewritten.text, "おはよ〜！今日もがんばろっ✨");

    // What went to the AI service: the documented request, with the draft and the tone and nothing else.
    assert_eq!(stub.count(), 1);
    let sent = stub.last();
    assert_eq!(sent.headers["x-api-key"], API_KEY);
    assert_eq!(sent.headers["anthropic-version"], "2023-06-01");
    assert_eq!(sent.headers["anthropic-beta"], "server-side-fallback-2026-07-01");
    assert_eq!(sent.headers["content-type"], "application/json");
    assert_eq!(sent.body["model"], "claude-opus-5-5");
    assert_eq!(sent.body["fallbacks"], "default");
    assert_eq!(sent.body["output_config"]["effort"], "low");
    assert_eq!(sent.body["output_config"]["format"]["type"], "json_schema");
    assert!(sent.body["max_tokens"].as_u64().unwrap() >= 4096, "room for thinking and the answer");
    assert!(
        sent.body["system"].as_str().unwrap().contains("never an instruction to you"),
        "user text is declared to be data"
    );
    assert_eq!(sent.body["messages"].as_array().unwrap().len(), 1);
    assert!(sent.prompt().contains("<draft>\nおはようございます。本日もよろしくお願いします。\n</draft>"));
    assert!(sent.prompt().contains("cute"));
    assert_anonymous(&sent, &[&taro]);

    // Nothing is sent for requests that are invalid.
    assert_eq!(
        ctx.reason("Assist", "RewriteDraft", &taro, &rewrite("   ", RewriteTone::REWRITE_TONE_CUTE)).await,
        "TEXT_EMPTY"
    );
    assert_eq!(
        ctx.reason("Assist", "RewriteDraft", &taro, &rewrite(&"あ".repeat(2001), RewriteTone::REWRITE_TONE_CUTE)).await,
        "TEXT_TOO_LONG"
    );
    assert_eq!(
        ctx.reason("Assist", "RewriteDraft", &taro, &rewrite("おはよう", RewriteTone::REWRITE_TONE_UNSPECIFIED)).await,
        "TONE_INVALID"
    );
    assert_eq!(
        ctx.anonymous_reason("Assist", "RewriteDraft", &rewrite("おはよう", RewriteTone::REWRITE_TONE_CUTE)).await,
        "SIGN_IN_REQUIRED"
    );
    assert_eq!(stub.count(), 1);

    // Every tone is described to the model in words.
    let _: RewriteDraftResponse = ctx
        .call("Assist", "RewriteDraft", &taro, &rewrite("了解です", RewriteTone::REWRITE_TONE_KANSAI))
        .await
        .unwrap();
    assert!(stub.last().prompt().contains("関西弁"));
}

#[tokio::test]
async fn replies_are_suggested_from_the_latest_messages() {
    let (ctx, stub) = start_with_stub(|_| {}).await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let direct = ctx.direct(&taro, &hanako).await;

    // Nothing to reply to: no request is made at all.
    let nothing: SuggestRepliesResponse =
        ctx.call("Assist", "SuggestReplies", &taro, &suggest(&direct.id)).await.unwrap();
    assert!(nothing.replies.is_empty());
    ctx.text(&taro, &direct.id, "自分だけが話している").await;
    let nothing: SuggestRepliesResponse =
        ctx.call("Assist", "SuggestReplies", &taro, &suggest(&direct.id)).await.unwrap();
    assert!(nothing.replies.is_empty());
    assert_eq!(stub.count(), 0);

    ctx.text(&hanako, &direct.id, "明日ひま？").await;
    ctx.text(&taro, &direct.id, "ひまだよ").await;
    let secret = ctx.text(&hanako, &direct.id, "やっぱり消すメッセージ").await;
    let _: DeleteMessageResponse = ctx
        .call(
            "Message",
            "DeleteMessage",
            &hanako,
            &DeleteMessageRequest { message_id: secret.id, ..Default::default() },
        )
        .await
        .unwrap();
    let photo = ctx.photo(&hanako).await;
    let with_photo = SendMessageRequest {
        conversation_id: direct.id.clone(),
        client_id: client_id(),
        attachment_asset_ids: vec![photo.clone()],
        ..Default::default()
    };
    ctx.send(&hanako, &with_photo).await.unwrap();
    ctx.text(&hanako, &direct.id, "映画いく？").await;

    let suggested: SuggestRepliesResponse =
        ctx.call("Assist", "SuggestReplies", &taro, &suggest(&direct.id)).await.unwrap();
    assert_eq!(suggested.replies, vec!["いいね！行こう", "何時から？", "ごめん、その日は無理かも"], "at most three");

    // Only the text of the latest messages was sent, labelled "me" / "other".
    let sent = stub.last();
    assert_eq!(
        sent.prompt(),
        "<conversation>\nme: 自分だけが話している\nother: 明日ひま？\nme: ひまだよ\nother: 映画いく？\n</conversation>"
    );
    assert_anonymous(&sent, &[&taro, &hanako]);
    assert!(!sent.body.to_string().contains(&photo), "no attachments");
    assert!(!sent.body.to_string().contains(&direct.id), "no conversation ID");

    // From the other side the labels swap.
    let _: SuggestRepliesResponse = ctx.call("Assist", "SuggestReplies", &hanako, &suggest(&direct.id)).await.unwrap();
    assert!(stub.last().prompt().ends_with("me: 映画いく？\n</conversation>"));

    let outsider = ctx.app.user("outsider").await;
    assert_eq!(ctx.reason("Assist", "SuggestReplies", &outsider, &suggest(&direct.id)).await, "CONVERSATION_NOT_FOUND");
    assert_eq!(ctx.anonymous_reason("Assist", "SuggestReplies", &suggest(&direct.id)).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn a_message_is_translated_and_the_translation_remembered() {
    let (ctx, stub) = start_with_stub(|_| {}).await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let direct = ctx.direct(&taro, &hanako).await;
    let message = ctx.text(&hanako, &direct.id, "おはよう！").await;

    let translated: TranslateMessageResponse =
        ctx.call("Assist", "TranslateMessage", &taro, &translate(&message.id, "EN-us")).await.unwrap();
    assert_eq!(translated.text, "Good morning!");
    assert_eq!((translated.source_language.as_str(), translated.target_language.as_str()), ("ja", "en-US"));
    let sent = stub.last();
    assert_eq!(sent.prompt(), "Target language: en-US\n\n<message>\nおはよう！\n</message>");
    assert_anonymous(&sent, &[&taro, &hanako]);
    assert!(!sent.body.to_string().contains(&message.id), "no message ID");

    // Asked again (by anyone in the conversation): answered from the cache.
    let again: TranslateMessageResponse =
        ctx.call("Assist", "TranslateMessage", &hanako, &translate(&message.id, "en-US")).await.unwrap();
    assert_eq!(again.text, "Good morning!");
    assert_eq!(stub.count(), 1);
    // Another language, or an edited text, is a new translation.
    let _: TranslateMessageResponse =
        ctx.call("Assist", "TranslateMessage", &taro, &translate(&message.id, "ko")).await.unwrap();
    assert_eq!(stub.count(), 2);
    let edit = EditMessageRequest {
        message_id: message.id.clone(),
        text: "おはよう！いい天気だね".into(),
        ..Default::default()
    };
    let _: EditMessageResponse = ctx.call("Message", "EditMessage", &hanako, &edit).await.unwrap();
    let _: TranslateMessageResponse =
        ctx.call("Assist", "TranslateMessage", &taro, &translate(&message.id, "en-US")).await.unwrap();
    assert_eq!(stub.count(), 3);
    assert!(stub.last().prompt().contains("いい天気だね"));

    // Without a target the language of the caller's account is used.
    sqlx::query("UPDATE users SET locale = 'ko-KR' WHERE id = $1")
        .bind(taro.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let own: TranslateMessageResponse =
        ctx.call("Assist", "TranslateMessage", &taro, &translate(&message.id, "")).await.unwrap();
    assert_eq!(own.target_language, "ko-KR");

    assert_eq!(
        ctx.reason("Assist", "TranslateMessage", &taro, &translate(&message.id, "english, please")).await,
        "LANGUAGE_INVALID"
    );
    let stamp_id = ctx.stamp(&hanako).await;
    let stamp = ctx
        .send(
            &hanako,
            &SendMessageRequest {
                conversation_id: direct.id.clone(),
                client_id: client_id(),
                stamp_id,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        ctx.reason("Assist", "TranslateMessage", &taro, &translate(&stamp.message.id, "en")).await,
        "NOTHING_TO_TRANSLATE"
    );
    let outsider = ctx.app.user("outsider").await;
    assert_eq!(
        ctx.reason("Assist", "TranslateMessage", &outsider, &translate(&message.id, "en")).await,
        "MESSAGE_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason("Assist", "TranslateMessage", &taro, &translate(&Uuid::now_v7().to_string(), "en")).await,
        "MESSAGE_NOT_FOUND"
    );
    assert_eq!(
        ctx.anonymous_reason("Assist", "TranslateMessage", &translate(&message.id, "en")).await,
        "SIGN_IN_REQUIRED"
    );
}

#[tokio::test]
async fn failures_of_the_ai_service_are_reported_as_such() {
    let (ctx, stub) = start_with_stub(|_| {}).await;
    let taro = ctx.app.user("taro").await;
    let tone = RewriteTone::REWRITE_TONE_POLITE;

    // The model declined: a refusal is not an answer.
    assert_eq!(ctx.reason("Assist", "RewriteDraft", &taro, &rewrite("REFUSE this", tone)).await, "AI_REFUSED");

    // Overloaded twice, then fine: retried without the user noticing.
    let before = stub.count();
    let recovered: RewriteDraftResponse =
        ctx.call("Assist", "RewriteDraft", &taro, &rewrite("FLAKY service", tone)).await.unwrap();
    assert!(!recovered.text.is_empty());
    assert_eq!(stub.count() - before, 3);

    // Overloaded for good: given up after three attempts.
    let before = stub.count();
    let error = ctx
        .call::<_, RewriteDraftResponse>("Assist", "RewriteDraft", &taro, &rewrite("OVERLOADED service", tone))
        .await
        .unwrap_err();
    assert_eq!((error.reason.as_str(), error.code.as_str()), ("AI_UNAVAILABLE", "unavailable"));
    assert_eq!(stub.count() - before, 3);

    // A rejected request (a wrong key) is not retried.
    let before = stub.count();
    assert_eq!(ctx.reason("Assist", "RewriteDraft", &taro, &rewrite("BADKEY", tone)).await, "AI_UNAVAILABLE");
    assert_eq!(stub.count() - before, 1);
}

#[tokio::test]
async fn an_unreachable_ai_service_is_unavailable() {
    // A port nothing listens on.
    let closed = {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap()
    };
    let cfg = ChatConfig {
        anthropic_api_key: API_KEY.into(),
        anthropic_base_url: format!("http://{closed}"),
        ai_timeout_seconds: 5,
        ai_allow_loopback: true,
        ..ChatConfig::default()
    };
    let ctx = start_with(cfg).await;
    let taro = ctx.app.user("taro").await;
    assert_eq!(
        ctx.reason("Assist", "RewriteDraft", &taro, &rewrite("おはよう", RewriteTone::REWRITE_TONE_COOL)).await,
        "AI_UNAVAILABLE"
    );
}

#[tokio::test]
async fn assistant_use_is_limited_per_user() {
    let (ctx, stub) = start_with_stub(|cfg| {
        cfg.ai_hourly_limit = 2;
        // A deployment on a model without server-side fallbacks or effort levels.
        cfg.ai_refusal_fallback = false;
        cfg.ai_effort = String::new();
        cfg.ai_model = "claude-haiku-5-5".into();
    })
    .await;
    let (taro, hanako) = (ctx.app.user("taro").await, ctx.app.user("hanako").await);
    let request = rewrite("おはよう", RewriteTone::REWRITE_TONE_SHORTER);

    for _ in 0..2 {
        let _: RewriteDraftResponse = ctx.call("Assist", "RewriteDraft", &taro, &request).await.unwrap();
    }
    let error = ctx.call::<_, RewriteDraftResponse>("Assist", "RewriteDraft", &taro, &request).await.unwrap_err();
    assert_eq!((error.reason.as_str(), error.code.as_str()), ("RATE_LIMITED", "resource_exhausted"));
    assert_eq!(stub.count(), 2, "a limited request is not sent");
    let _: RewriteDraftResponse =
        ctx.call("Assist", "RewriteDraft", &hanako, &request).await.expect("the limit is per user");

    // The configuration decides model, effort and fallbacks.
    let sent = stub.last();
    assert_eq!(sent.body["model"], "claude-haiku-5-5");
    assert!(sent.body.get("fallbacks").is_none() && !sent.headers.contains_key("anthropic-beta"));
    assert!(sent.body["output_config"].get("effort").is_none());
}

async fn settings_of(ctx: &Ctx, user: &TestUser) -> AssistSettings {
    let response: GetAssistSettingsResponse =
        ctx.call("Assist", "GetAssistSettings", user, &GetAssistSettingsRequest::default()).await.unwrap();
    response.settings.into_option().expect("settings")
}

async fn set_birthdate(ctx: &Ctx, user: &TestUser, birthdate: &str) {
    sqlx::query("UPDATE users SET birthdate = $2::date WHERE id = $1")
        .bind(user.id)
        .bind(birthdate)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
}

async fn consent(ctx: &Ctx, user: &TestUser, allow: bool) -> AssistSettings {
    let response: SetAiMessageConsentResponse = ctx
        .call("Assist", "SetAiMessageConsent", user, &SetAiMessageConsentRequest { allow, ..Default::default() })
        .await
        .unwrap();
    response.settings.into_option().unwrap()
}

/// Under 18 today, whatever the date of the test run.
fn minor_birthdate() -> String {
    let today = chrono::Utc::now().date_naive();
    format!("{}-01-01", chrono::Datelike::year(&today) - 15)
}

#[tokio::test]
async fn other_peoples_messages_go_to_the_ai_only_with_their_consent() {
    let (ctx, stub) = start_with_stub(|_| {}).await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let jiro = ctx.app.user("jiro").await;
    set_birthdate(&ctx, &hanako, &minor_birthdate()).await;

    // The defaults: off under 18, on for adults, and said to be the default.
    let minor = settings_of(&ctx, &hanako).await;
    assert!(!minor.allow_message_use && minor.allow_message_use_is_default);
    let adult = settings_of(&ctx, &taro).await;
    assert!(adult.allow_message_use && adult.allow_message_use_is_default);

    let direct = ctx.direct(&taro, &hanako).await;
    let hello = ctx.text(&hanako, &direct.id, "明日ひま？").await;
    ctx.text(&taro, &direct.id, "ひまだよ").await;

    // Hanako (under 18, never chose) is not sent: nothing left to reply to, no request.
    let suggested: SuggestRepliesResponse =
        ctx.call("Assist", "SuggestReplies", &taro, &suggest(&direct.id)).await.unwrap();
    assert!(suggested.replies.is_empty());
    assert_eq!(suggested.skipped_messages, 1);
    assert_eq!(
        ctx.reason("Assist", "TranslateMessage", &taro, &translate(&hello.id, "en")).await,
        "SENDER_DISALLOWS_AI"
    );
    assert_eq!(stub.count(), 0, "nothing of hanako's left the server");

    // Her own message is hers to translate, and her own replies use only what may be sent.
    let own: TranslateMessageResponse =
        ctx.call("Assist", "TranslateMessage", &hanako, &translate(&hello.id, "en")).await.unwrap();
    assert_eq!(own.text, "Good morning!");
    assert_eq!(stub.count(), 1);

    // Once she allows it, her messages are used.
    let allowed = consent(&ctx, &hanako, true).await;
    assert!(allowed.allow_message_use && !allowed.allow_message_use_is_default);
    let suggested: SuggestRepliesResponse =
        ctx.call("Assist", "SuggestReplies", &taro, &suggest(&direct.id)).await.unwrap();
    assert_eq!(suggested.skipped_messages, 0);
    assert_eq!(stub.last().prompt(), "<conversation>\nother: 明日ひま？\nme: ひまだよ\n</conversation>");

    // An adult who says no is left out the same way, in a group too.
    consent(&ctx, &taro, false).await;
    ctx.befriend(&hanako, &jiro).await;
    let group_id = ctx.group(&hanako, "放課後", &[&taro, &jiro]).await.id;
    let taros = ctx.text(&taro, &group_id, "太郎の秘密").await;
    ctx.text(&hanako, &group_id, "みんなで行こう").await;
    let suggested: SuggestRepliesResponse =
        ctx.call("Assist", "SuggestReplies", &jiro, &suggest(&group_id)).await.unwrap();
    assert_eq!(suggested.skipped_messages, 1);
    let sent = stub.last();
    assert_eq!(sent.prompt(), "<conversation>\nother: みんなで行こう\n</conversation>");
    assert!(!sent.body.to_string().contains("太郎の秘密"));
    assert_eq!(
        ctx.reason("Assist", "TranslateMessage", &jiro, &translate(&taros.id, "en")).await,
        "SENDER_DISALLOWS_AI"
    );

    // A cached translation is no way around a "no" either.
    let cached = ctx.text(&hanako, &direct.id, "キャッシュされる").await;
    let _: TranslateMessageResponse =
        ctx.call("Assist", "TranslateMessage", &taro, &translate(&cached.id, "en")).await.unwrap();
    consent(&ctx, &hanako, false).await;
    assert_eq!(
        ctx.reason("Assist", "TranslateMessage", &taro, &translate(&cached.id, "en")).await,
        "SENDER_DISALLOWS_AI"
    );
}

#[tokio::test]
async fn users_choose_among_the_enabled_providers_and_can_switch_the_assistant_off() {
    let (ctx, stub) = start_with_stub(|_| {}).await;
    let taro = ctx.app.user("taro").await;

    let settings = settings_of(&ctx, &taro).await;
    assert_eq!(settings.providers.len(), 1, "only Anthropic has a key in this environment");
    let option = &settings.providers[0];
    assert_eq!(option.provider.as_known(), Some(AiProvider::AI_PROVIDER_ANTHROPIC));
    // The base URL is the stub, not Anthropic's API: the stub's host is what receives the text.
    assert_eq!(option.recipient, "127.0.0.1");
    assert_eq!(option.model, "claude-opus-5-5");
    assert!(option.is_default && option.usable && option.accepts_own_key);
    assert_eq!(settings.effective_provider.as_known(), Some(AiProvider::AI_PROVIDER_ANTHROPIC));
    assert_eq!(settings.effective_recipient, "127.0.0.1");

    let choose = |provider: AiProvider, off: bool| ChooseAssistProviderRequest {
        provider: provider.into(),
        off,
        ..Default::default()
    };
    assert_eq!(
        ctx.reason("Assist", "ChooseAssistProvider", &taro, &choose(AiProvider::AI_PROVIDER_OPENAI, false)).await,
        "PROVIDER_NOT_AVAILABLE"
    );

    // Off: every assistant RPC says so, and the capabilities agree.
    let off: ChooseAssistProviderResponse = ctx
        .call("Assist", "ChooseAssistProvider", &taro, &choose(AiProvider::AI_PROVIDER_UNSPECIFIED, true))
        .await
        .unwrap();
    let off = off.settings.into_option().unwrap();
    assert!(off.off && off.effective_provider.as_known() == Some(AiProvider::AI_PROVIDER_UNSPECIFIED));
    assert_eq!(
        ctx.reason("Assist", "RewriteDraft", &taro, &rewrite("おはよう", RewriteTone::REWRITE_TONE_CUTE)).await,
        "AI_TURNED_OFF"
    );
    let capabilities: GetCapabilitiesResponse =
        ctx.call("Conversation", "GetCapabilities", &taro, &GetCapabilitiesRequest::default()).await.unwrap();
    assert!(!capabilities.capabilities.assist_enabled);
    assert_eq!(stub.count(), 0);

    let _: ChooseAssistProviderResponse = ctx
        .call("Assist", "ChooseAssistProvider", &taro, &choose(AiProvider::AI_PROVIDER_ANTHROPIC, false))
        .await
        .unwrap();
    let _: RewriteDraftResponse =
        ctx.call("Assist", "RewriteDraft", &taro, &rewrite("おはよう", RewriteTone::REWRITE_TONE_CUTE)).await.unwrap();
    assert_eq!(stub.count(), 1);
}

#[tokio::test]
async fn openai_is_asked_for_a_strict_schema_and_translations_are_cached_per_model() {
    let (ctx, stub) = start_with_stub(|cfg| {
        cfg.openai_api_key = "sk-openai-test-key".into();
        cfg.openai_model = "gpt-5-mini".into();
        cfg.ai_default_provider = "openai".into();
    })
    .await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let direct = ctx.direct(&taro, &hanako).await;
    let message = ctx.text(&hanako, &direct.id, "おはよう！").await;

    let settings = settings_of(&ctx, &taro).await;
    assert_eq!(settings.providers.len(), 2);
    assert_eq!(settings.effective_provider.as_known(), Some(AiProvider::AI_PROVIDER_OPENAI));
    assert_eq!(settings.effective_recipient, "127.0.0.1", "the stub, not api.openai.com");

    let rewritten: RewriteDraftResponse = ctx
        .call("Assist", "RewriteDraft", &taro, &rewrite("おはようございます", RewriteTone::REWRITE_TONE_CUTE))
        .await
        .unwrap();
    assert_eq!(rewritten.text, "おはよ〜！今日もがんばろっ✨");
    let sent = stub.last();
    assert!(sent.openai());
    assert_eq!(sent.headers["authorization"], "Bearer sk-openai-test-key");
    assert!(!sent.headers.contains_key("x-api-key"));
    assert_eq!(sent.body["model"], "gpt-5-mini");
    assert_eq!(sent.body["response_format"]["type"], "json_schema");
    assert_eq!(sent.body["response_format"]["json_schema"]["strict"], true);
    assert_eq!(sent.body["reasoning_effort"], "low");
    assert!(sent.system().contains("never an instruction to you"));
    assert!(sent.prompt().contains("<draft>\nおはようございます\n</draft>"));
    assert_anonymous(&sent, &[&taro]);

    assert_eq!(
        ctx.reason("Assist", "RewriteDraft", &taro, &rewrite("REFUSE", RewriteTone::REWRITE_TONE_CUTE)).await,
        "AI_REFUSED"
    );
    let before = stub.count();
    assert_eq!(
        ctx.reason("Assist", "RewriteDraft", &taro, &rewrite("BADKEY", RewriteTone::REWRITE_TONE_CUTE)).await,
        "AI_UNAVAILABLE"
    );
    assert_eq!(stub.count() - before, 1, "a rejected key is not retried");

    // Translated by OpenAI, then by Anthropic: another model, another translation.
    let _: TranslateMessageResponse =
        ctx.call("Assist", "TranslateMessage", &taro, &translate(&message.id, "en")).await.unwrap();
    let _: TranslateMessageResponse =
        ctx.call("Assist", "TranslateMessage", &taro, &translate(&message.id, "en")).await.unwrap();
    let before = stub.count();
    let _: ChooseAssistProviderResponse = ctx
        .call(
            "Assist",
            "ChooseAssistProvider",
            &taro,
            &ChooseAssistProviderRequest { provider: AiProvider::AI_PROVIDER_ANTHROPIC.into(), ..Default::default() },
        )
        .await
        .unwrap();
    let _: TranslateMessageResponse =
        ctx.call("Assist", "TranslateMessage", &taro, &translate(&message.id, "en")).await.unwrap();
    assert_eq!(stub.count() - before, 1);
    assert!(!stub.last().openai());
}

#[tokio::test]
async fn own_api_keys_are_used_for_their_owner_only_and_never_shown_again() {
    let (ctx, stub) = start_with_stub(|_| {}).await;
    let (taro, hanako) = (ctx.app.user("taro").await, ctx.app.user("hanako").await);
    let own_key = "sk-ant-taro-own-key-WXYZ";

    let set = |provider: AiProvider, key: &str| SetOwnApiKeyRequest {
        provider: provider.into(),
        api_key: key.into(),
        ..Default::default()
    };
    assert_eq!(
        ctx.reason("Assist", "SetOwnApiKey", &taro, &set(AiProvider::AI_PROVIDER_OPENAI_COMPATIBLE, own_key)).await,
        "PROVIDER_INVALID"
    );
    assert_eq!(
        ctx.reason("Assist", "SetOwnApiKey", &taro, &set(AiProvider::AI_PROVIDER_ANTHROPIC, "short")).await,
        "API_KEY_INVALID"
    );
    assert_eq!(
        ctx.reason("Assist", "SetOwnApiKey", &taro, &set(AiProvider::AI_PROVIDER_ANTHROPIC, "has a space in it")).await,
        "API_KEY_INVALID"
    );
    let saved: SetOwnApiKeyResponse =
        ctx.call("Assist", "SetOwnApiKey", &taro, &set(AiProvider::AI_PROVIDER_ANTHROPIC, own_key)).await.unwrap();
    let saved = saved.settings.into_option().unwrap();
    assert_eq!(saved.own_keys.len(), 1);
    assert_eq!(saved.own_keys[0].last4, "WXYZ");
    // The key is never sent back, in any answer.
    let shown = serde_json::to_string(&settings_of(&ctx, &taro).await).unwrap();
    assert!(!shown.contains(own_key) && !shown.contains("taro-own"));
    // At rest it is sealed.
    let stored: Vec<u8> = sqlx::query_scalar("SELECT key_sealed FROM chat_ai_user_keys WHERE user_id = $1")
        .bind(taro.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&stored).contains("taro-own-key"));

    // Taro's requests carry his key; Hanako's the operator's.
    let request = rewrite("おはよう", RewriteTone::REWRITE_TONE_SHORTER);
    let _: RewriteDraftResponse = ctx.call("Assist", "RewriteDraft", &taro, &request).await.unwrap();
    assert_eq!(stub.last().headers["x-api-key"], own_key);
    let _: RewriteDraftResponse = ctx.call("Assist", "RewriteDraft", &hanako, &request).await.unwrap();
    assert_eq!(stub.last().headers["x-api-key"], API_KEY);

    // Removed: back to the operator's.
    let removed: RemoveOwnApiKeyResponse = ctx
        .call(
            "Assist",
            "RemoveOwnApiKey",
            &taro,
            &RemoveOwnApiKeyRequest { provider: AiProvider::AI_PROVIDER_ANTHROPIC.into(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(removed.settings.into_option().unwrap().own_keys.is_empty());
    let _: RewriteDraftResponse = ctx.call("Assist", "RewriteDraft", &taro, &request).await.unwrap();
    assert_eq!(stub.last().headers["x-api-key"], API_KEY);

    // Key changes are limited per user.
    let mut limited = None;
    for n in 0..12 {
        let request = set(AiProvider::AI_PROVIDER_ANTHROPIC, &format!("sk-ant-try-{n:04}"));
        if let Err(error) = ctx.call::<_, SetOwnApiKeyResponse>("Assist", "SetOwnApiKey", &hanako, &request).await {
            limited = Some((n, error.reason));
            break;
        }
    }
    assert_eq!(limited, Some((10, "RATE_LIMITED".to_owned())));
}

#[tokio::test]
async fn ai_requests_go_through_the_outbound_guard() {
    // The same stub, but loopback is not allowed: the request is refused before it leaves.
    let (stub, address) = start_stub().await;
    let cfg = ChatConfig {
        anthropic_api_key: API_KEY.into(),
        anthropic_base_url: format!("http://{address}"),
        ai_timeout_seconds: 5,
        ..ChatConfig::default()
    };
    let ctx = start_with(cfg).await;
    let taro = ctx.app.user("taro").await;
    assert_eq!(
        ctx.reason("Assist", "RewriteDraft", &taro, &rewrite("おはよう", RewriteTone::REWRITE_TONE_COOL)).await,
        "AI_UNAVAILABLE"
    );
    assert_eq!(stub.count(), 0);
}

#[tokio::test]
async fn a_deleted_account_takes_its_ai_keys_and_settings_along() {
    let (ctx, _stub) = start_with_stub(|_| {}).await;
    let taro = ctx.app.user("taro").await;
    let key = SetOwnApiKeyRequest {
        provider: AiProvider::AI_PROVIDER_ANTHROPIC.into(),
        api_key: "sk-ant-deleted-user-key".into(),
        ..Default::default()
    };
    let _: SetOwnApiKeyResponse = ctx.call("Assist", "SetOwnApiKey", &taro, &key).await.unwrap();
    consent(&ctx, &taro, true).await;
    let event = tokuly_core::events::Event {
        id: Uuid::now_v7(),
        topic: "identity.user.deleted".into(),
        actor_id: None,
        occurred_at: chrono::Utc::now(),
        data: serde_json::json!({ "user_id": taro.id }),
    };
    ctx.chat.handle_event(&event).await.unwrap();
    for table in ["chat_ai_user_keys", "chat_ai_user_settings"] {
        let left: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table} WHERE user_id = $1")))
                .bind(taro.id)
                .fetch_one(&ctx.app.core.db)
                .await
                .unwrap();
        assert_eq!(left, 0, "{table}");
    }
}

#[tokio::test]
async fn settings_saved_at_once_from_one_account_end_up_as_one_row() {
    let (ctx, _stub) = start_with_stub(|_| {}).await;
    let taro = ctx.app.user("taro").await;
    let keys = (0..6).map(|n| SetOwnApiKeyRequest {
        provider: AiProvider::AI_PROVIDER_ANTHROPIC.into(),
        api_key: format!("sk-ant-concurrent-{n:04}"),
        ..Default::default()
    });
    let consents = (0..6).map(|n| SetAiMessageConsentRequest { allow: n % 2 == 0, ..Default::default() });
    let saved = futures::future::join_all(keys.map(|request| {
        let ctx = &ctx;
        let taro = &taro;
        async move { ctx.call::<_, SetOwnApiKeyResponse>("Assist", "SetOwnApiKey", taro, &request).await }
    }));
    let consented =
        futures::future::join_all(
            consents.map(|request| {
                let ctx = &ctx;
                let taro = &taro;
                async move {
                    ctx.call::<_, SetAiMessageConsentResponse>("Assist", "SetAiMessageConsent", taro, &request).await
                }
            }),
        );
    let (saved, consented) = tokio::join!(saved, consented);
    assert!(saved.iter().all(Result::is_ok) && consented.iter().all(Result::is_ok));
    let settings = settings_of(&ctx, &taro).await;
    assert_eq!(settings.own_keys.len(), 1, "one key per provider, whatever the order");
    assert!(!settings.allow_message_use_is_default);
}

/// A date `years` years and `days` days before today.
fn born(years: i32, days: i64) -> String {
    let today = chrono::Utc::now().date_naive();
    use chrono::Datelike as _;
    let anniversary =
        chrono::NaiveDate::from_ymd_opt(today.year() - years, today.month(), today.day().min(28)).unwrap();
    (anniversary - chrono::Duration::days(days)).to_string()
}

#[tokio::test]
async fn the_consent_default_is_fixed_when_the_account_is_created_and_never_moves_with_a_birthday() {
    let (ctx, stub) = start_with_stub(|_| {}).await;

    // Signed up at 17: the event stores "off".
    let (taro, minor) = ctx.friends("taro", "minor").await;
    set_birthdate(&ctx, &minor, &born(17, 0)).await;
    let created = tokuly_core::events::Event {
        id: Uuid::now_v7(),
        topic: "identity.user.created".into(),
        actor_id: Some(minor.id),
        occurred_at: chrono::Utc::now(),
        data: serde_json::json!({ "user_id": minor.id, "handle": minor.handle, "invite_code": "" }),
    };
    ctx.chat.handle_event(&created).await.unwrap();
    ctx.chat.handle_event(&created).await.unwrap();
    // Their 18th birthday passes (here: the birthdate moves back past it).
    set_birthdate(&ctx, &minor, &born(18, 30)).await;
    let settings = settings_of(&ctx, &minor).await;
    assert!(!settings.allow_message_use && settings.allow_message_use_is_default, "still off after turning 18");
    let direct = ctx.direct(&taro, &minor).await;
    let message = ctx.text(&minor, &direct.id, "まだないしょ").await;
    assert_eq!(
        ctx.reason("Assist", "TranslateMessage", &taro, &translate(&message.id, "en")).await,
        "SENDER_DISALLOWS_AI",
        "nothing anybody sees changes with the birthday"
    );

    // An account whose event never arrived: judged by its age when it was created, not today.
    let late = ctx.app.user("late").await;
    sqlx::query("UPDATE users SET created_at = now() - interval '2 years', birthdate = $2::date WHERE id = $1")
        .bind(late.id)
        .bind(born(19, 0))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert!(!settings_of(&ctx, &late).await.allow_message_use, "17 when the account was created");

    // An adult starts on; a choice wins over the stored default, and over a late event.
    let adult = ctx.app.user("adult").await;
    ctx.chat
        .handle_event(&tokuly_core::events::Event {
            data: serde_json::json!({ "user_id": adult.id, "handle": adult.handle, "invite_code": "" }),
            ..created.clone()
        })
        .await
        .unwrap();
    let settings = settings_of(&ctx, &adult).await;
    assert!(settings.allow_message_use && settings.allow_message_use_is_default);
    consent(&ctx, &minor, true).await;
    ctx.chat.handle_event(&created).await.unwrap();
    let chosen = settings_of(&ctx, &minor).await;
    assert!(chosen.allow_message_use && !chosen.allow_message_use_is_default);
    let _: TranslateMessageResponse =
        ctx.call("Assist", "TranslateMessage", &taro, &translate(&message.id, "en")).await.unwrap();
    assert_eq!(stub.count(), 1);
}

#[tokio::test]
async fn a_translation_is_no_longer_than_a_message() {
    let (ctx, stub) = start_with_stub(|_| {}).await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let direct = ctx.direct(&taro, &hanako).await;
    let message = ctx.text(&hanako, &direct.id, "LONGANSWER please").await;
    let max = tokuly_chat::content::MAX_TEXT_CHARS;
    let first: TranslateMessageResponse =
        ctx.call("Assist", "TranslateMessage", &taro, &translate(&message.id, "en")).await.unwrap();
    assert_eq!(first.text.chars().count(), max);
    // The cache holds the capped text too.
    let again: TranslateMessageResponse =
        ctx.call("Assist", "TranslateMessage", &hanako, &translate(&message.id, "en")).await.unwrap();
    assert_eq!(stub.count(), 1, "answered from the cache");
    assert_eq!(again.text.chars().count(), max);
}
