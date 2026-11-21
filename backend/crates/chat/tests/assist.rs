//! `AssistService`, against a local stand-in for the Anthropic Messages
//! API (`TOKULY_ANTHROPIC_BASE_URL`). The stub records what the server
//! sends, so the tests can check that only the text a feature needs leaves
//! the server. Nothing here talks to the real API.
mod common;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
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
        self.body["messages"][0]["content"].as_str().unwrap_or_default()
    }
}

#[derive(Clone, Default)]
struct Stub {
    requests: Arc<Mutex<Vec<Recorded>>>,
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

/// Answers like the Messages API would, steered by marker words in the prompt.
async fn messages(
    State(stub): State<Stub>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let headers = headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), value.to_str().unwrap_or_default().to_owned()))
        .collect();
    let recorded = Recorded { headers, body };
    let prompt = recorded.prompt().to_owned();
    let system = recorded.body["system"].as_str().unwrap_or_default().to_owned();
    let attempts = {
        let mut requests = stub.requests.lock().unwrap();
        requests.push(recorded);
        requests.iter().filter(|request| request.prompt() == prompt).count()
    };
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
    let answer = if system.contains("rewrite") {
        serde_json::json!({ "text": "おはよ〜！今日もがんばろっ✨" })
    } else if system.contains("quick replies") {
        serde_json::json!({ "replies": ["いいね！行こう", "何時から？", "ごめん、その日は無理かも", "四つ目は多すぎ"] })
    } else {
        serde_json::json!({ "translation": "Good morning!", "source_language": "JA" })
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

/// Starts the stub and a chat domain whose assistant talks to it.
async fn start_with_stub(change: impl FnOnce(&mut ChatConfig)) -> (Ctx, Stub) {
    let stub = Stub::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
    let address = listener.local_addr().unwrap();
    let router = Router::new().route("/v1/messages", post(messages)).with_state(stub.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.expect("stub server") });
    let mut cfg = ChatConfig {
        anthropic_api_key: API_KEY.into(),
        anthropic_base_url: format!("http://{address}/"),
        ai_timeout_seconds: 15,
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
