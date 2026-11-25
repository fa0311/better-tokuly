//! `AssistAdminService`: the operator's AI provider settings. They are
//! global to the instance, so the tests of this binary take turns
//! ([`TURN`]) and start from a clean slate. The compatible provider runs
//! against a local stand-in for an OpenAI-compatible server (like LM
//! Studio) that does not support `response_format`, on loopback, reached
//! through the operator's private-host allowance.
mod common;

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use axum::{Json, Router, extract::State, http::HeaderMap, http::StatusCode, routing::post};
use common::*;
use tokuly_chat::ChatConfig;
use tokuly_proto::chat::*;
use tokuly_testkit::TestUser;

/// The operator settings are one per instance: one test at a time.
static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const ENV_KEY: &str = "sk-ant-environment-key-1111";

#[derive(Clone, Default)]
struct Stub {
    requests: Arc<Mutex<Vec<(HeaderMap, serde_json::Value)>>>,
    supports_schema: Arc<AtomicBool>,
}

impl Stub {
    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    fn take(&self) -> Vec<(HeaderMap, serde_json::Value)> {
        std::mem::take(&mut *self.requests.lock().unwrap())
    }
}

/// A compatible server: answers plain JSON, and (unless told otherwise)
/// rejects `response_format` like a server without schema support.
async fn completions(
    State(stub): State<Stub>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    stub.requests.lock().unwrap().push((headers, body.clone()));
    if body.get("response_format").is_some() && !stub.supports_schema.load(Ordering::SeqCst) {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "response_format is not supported" })));
    }
    let content = "<think>short task</think>\n```json\n{\"text\": \"ローカルで書き直したよ！\"}\n```";
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": content }, "finish_reason": "stop" }]
        })),
    )
}

/// Anything sent to the Anthropic stand-in is a test failure waiting to be noticed.
async fn messages(State(stub): State<Stub>, headers: HeaderMap, Json(body): Json<serde_json::Value>) -> StatusCode {
    stub.requests.lock().unwrap().push((headers, body));
    StatusCode::INTERNAL_SERVER_ERROR
}

async fn start_stub() -> (Stub, std::net::SocketAddr) {
    let stub = Stub::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new()
        .route("/v1/chat/completions", post(completions))
        .route("/v1/messages", post(messages))
        .with_state(stub.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (stub, address)
}

/// A fresh instance: nothing saved, Anthropic on from the environment.
async fn fresh(cfg: ChatConfig) -> Ctx {
    let ctx = start_with(cfg).await;
    for table in ["chat_ai_providers", "chat_ai_instance"] {
        sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table}"))).execute(&ctx.app.core.db).await.unwrap();
    }
    ctx
}

fn env_config() -> ChatConfig {
    ChatConfig { anthropic_api_key: ENV_KEY.into(), ai_timeout_seconds: 10, ..ChatConfig::default() }
}

async fn instance(ctx: &Ctx, admin: &TestUser) -> AssistInstance {
    let response: GetAssistInstanceResponse =
        ctx.call("AssistAdmin", "GetAssistInstance", admin, &GetAssistInstanceRequest::default()).await.unwrap();
    response.instance.into_option().unwrap()
}

fn provider_of(instance: &AssistInstance, provider: AiProvider) -> AssistProviderConfig {
    instance.providers.iter().find(|config| config.provider.as_known() == Some(provider)).cloned().unwrap()
}

fn compat(base_url: &str, allow_private_host: bool) -> UpdateAssistProviderRequest {
    UpdateAssistProviderRequest {
        provider: AiProvider::AI_PROVIDER_OPENAI_COMPATIBLE.into(),
        enabled: true,
        model: "qwen3-8b".into(),
        base_url: base_url.into(),
        allow_private_host,
        ..Default::default()
    }
}

fn rewrite() -> RewriteDraftRequest {
    RewriteDraftRequest {
        text: "おはよう".into(), tone: RewriteTone::REWRITE_TONE_CASUAL.into(), ..Default::default()
    }
}

#[tokio::test]
async fn only_admins_see_and_change_the_operator_settings_and_keys_stay_secret() {
    let _turn = TURN.lock().await;
    let ctx = fresh(env_config()).await;
    let admin = ctx.app.user_with_roles("admin", &["admin"]).await;
    let moderator = ctx.app.user_with_roles("moderator", &["moderator"]).await;
    let taro = ctx.app.user("taro").await;
    for user in [&moderator, &taro] {
        assert_eq!(
            ctx.reason("AssistAdmin", "GetAssistInstance", user, &GetAssistInstanceRequest::default()).await,
            "FORBIDDEN"
        );
        assert_eq!(
            ctx.reason("AssistAdmin", "UpdateAssistProvider", user, &compat("http://127.0.0.1:1/v1", true)).await,
            "FORBIDDEN"
        );
    }

    // The environment's values, the key only as its last four characters.
    let start = instance(&ctx, &admin).await;
    let anthropic = provider_of(&start, AiProvider::AI_PROVIDER_ANTHROPIC);
    assert!(anthropic.enabled && anthropic.ready && anthropic.from_environment && anthropic.api_key_set);
    assert_eq!(anthropic.api_key_last4, "1111");
    assert_eq!(anthropic.model, "claude-opus-5-5");
    assert_eq!(anthropic.base_url, "https://api.anthropic.com");
    let openai = provider_of(&start, AiProvider::AI_PROVIDER_OPENAI);
    assert!(!openai.enabled && !openai.api_key_set);
    assert_eq!(openai.model, "gpt-5-mini");
    assert_eq!(start.effective_default.as_known(), Some(AiProvider::AI_PROVIDER_ANTHROPIC));
    assert!(!serde_json::to_string(&start).unwrap().contains("environment-key"));

    // Saving other fields keeps the environment's key; a new key replaces it.
    let save = |model: &str, change: ApiKeyChange, key: &str| UpdateAssistProviderRequest {
        provider: AiProvider::AI_PROVIDER_ANTHROPIC.into(),
        enabled: true,
        model: model.into(),
        base_url: String::new(),
        api_key_change: change.into(),
        api_key: key.into(),
        ..Default::default()
    };
    let saved: UpdateAssistProviderResponse = ctx
        .call(
            "AssistAdmin",
            "UpdateAssistProvider",
            &admin,
            &save("claude-haiku-5-5", ApiKeyChange::API_KEY_CHANGE_UNSPECIFIED, ""),
        )
        .await
        .unwrap();
    let anthropic = provider_of(&saved.instance.into_option().unwrap(), AiProvider::AI_PROVIDER_ANTHROPIC);
    assert!(!anthropic.from_environment && anthropic.api_key_set);
    assert_eq!((anthropic.model.as_str(), anthropic.api_key_last4.as_str()), ("claude-haiku-5-5", "1111"));
    let new_key = "sk-ant-console-secret-9876";
    let saved: UpdateAssistProviderResponse = ctx
        .call(
            "AssistAdmin",
            "UpdateAssistProvider",
            &admin,
            &save("claude-haiku-5-5", ApiKeyChange::API_KEY_CHANGE_SET, new_key),
        )
        .await
        .unwrap();
    let shown = serde_json::to_string(&saved).unwrap();
    assert!(shown.contains("\"9876\"") && !shown.contains("console-secret"), "never returned");
    let sealed: Vec<u8> = sqlx::query_scalar("SELECT api_key_sealed FROM chat_ai_providers WHERE provider = 1")
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&sealed).contains("console-secret"), "sealed at rest");
    // The record of the change names who and what, never the key.
    let recorded = ctx.app.events_by("chat.assist.", admin.id).await;
    assert_eq!(recorded.len(), 2);
    let payloads = serde_json::to_string(
        &recorded.iter().map(|event| event.data::<serde_json::Value>().unwrap()).collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(payloads.contains("\"set\"") && payloads.contains("\"kept\"") && !payloads.contains("console-secret"));

    let cleared: UpdateAssistProviderResponse = ctx
        .call(
            "AssistAdmin",
            "UpdateAssistProvider",
            &admin,
            &save("claude-haiku-5-5", ApiKeyChange::API_KEY_CHANGE_CLEAR, ""),
        )
        .await
        .unwrap();
    let anthropic = provider_of(&cleared.instance.into_option().unwrap(), AiProvider::AI_PROVIDER_ANTHROPIC);
    assert!(
        !anthropic.api_key_set && !anthropic.ready,
        "enabled, but without a key only users who bring one can use it"
    );

    // Validation.
    for (request, reason) in [
        (save("", ApiKeyChange::API_KEY_CHANGE_UNSPECIFIED, ""), "MODEL_INVALID"),
        (save("m", ApiKeyChange::API_KEY_CHANGE_SET, "short"), "API_KEY_INVALID"),
        (
            UpdateAssistProviderRequest {
                base_url: "https://user:pw@api.anthropic.com".into(),
                ..save("m", ApiKeyChange::API_KEY_CHANGE_UNSPECIFIED, "")
            },
            "BASE_URL_INVALID",
        ),
        (
            UpdateAssistProviderRequest {
                base_url: "http://10.0.0.5".into(),
                ..save("m", ApiKeyChange::API_KEY_CHANGE_UNSPECIFIED, "")
            },
            "BASE_URL_NOT_ALLOWED",
        ),
        (
            UpdateAssistProviderRequest {
                allow_private_host: true,
                ..save("m", ApiKeyChange::API_KEY_CHANGE_UNSPECIFIED, "")
            },
            "PRIVATE_HOST_NOT_ALLOWED",
        ),
        (
            UpdateAssistProviderRequest {
                provider: AiProvider::AI_PROVIDER_UNSPECIFIED.into(),
                ..save("m", ApiKeyChange::API_KEY_CHANGE_UNSPECIFIED, "")
            },
            "PROVIDER_INVALID",
        ),
    ] {
        assert_eq!(ctx.reason("AssistAdmin", "UpdateAssistProvider", &admin, &request).await, reason);
    }
    assert_eq!(
        ctx.reason(
            "AssistAdmin",
            "SetDefaultAssistProvider",
            &admin,
            &SetDefaultAssistProviderRequest { provider: AiProvider::AI_PROVIDER_OPENAI.into(), ..Default::default() }
        )
        .await,
        "PROVIDER_NOT_AVAILABLE"
    );
}

#[tokio::test]
async fn a_local_compatible_server_is_reached_through_the_allowance_with_plain_json() {
    let _turn = TURN.lock().await;
    let (stub, address) = start_stub().await;
    let ctx = fresh(env_config()).await;
    let admin = ctx.app.user_with_roles("admin", &["admin"]).await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let base_url = format!("http://{address}/v1");

    // A loopback address on its own port: refused without the allowance.
    assert_eq!(
        ctx.reason("AssistAdmin", "UpdateAssistProvider", &admin, &compat(&base_url, false)).await,
        "BASE_URL_NOT_ALLOWED"
    );
    let saved: UpdateAssistProviderResponse =
        ctx.call("AssistAdmin", "UpdateAssistProvider", &admin, &compat(&base_url, true)).await.unwrap();
    let config = provider_of(&saved.instance.into_option().unwrap(), AiProvider::AI_PROVIDER_OPENAI_COMPATIBLE);
    assert!(config.enabled && config.ready && config.allow_private_host && !config.api_key_set);
    let recorded = ctx.app.events_by("chat.assist.provider_updated", admin.id).await;
    let payload: serde_json::Value = recorded.last().unwrap().data().unwrap();
    assert_eq!(payload["private_host"], "127.0.0.1", "the exception is on record");

    // Users see who receives the text, and pick it.
    let settings: GetAssistSettingsResponse =
        ctx.call("Assist", "GetAssistSettings", &taro, &GetAssistSettingsRequest::default()).await.unwrap();
    let settings = settings.settings.into_option().unwrap();
    let local = settings
        .providers
        .iter()
        .find(|option| option.provider.as_known() == Some(AiProvider::AI_PROVIDER_OPENAI_COMPATIBLE))
        .unwrap();
    assert_eq!((local.recipient.as_str(), local.model.as_str()), ("127.0.0.1", "qwen3-8b"));
    assert!(local.usable && !local.accepts_own_key);
    let choose = ChooseAssistProviderRequest {
        provider: AiProvider::AI_PROVIDER_OPENAI_COMPATIBLE.into(),
        ..Default::default()
    };
    let chosen: ChooseAssistProviderResponse =
        ctx.call("Assist", "ChooseAssistProvider", &taro, &choose).await.unwrap();
    assert_eq!(chosen.settings.into_option().unwrap().effective_recipient, "127.0.0.1");

    // The server lacks schema support: asked again for plain JSON, read leniently.
    let rewritten: RewriteDraftResponse = ctx.call("Assist", "RewriteDraft", &taro, &rewrite()).await.unwrap();
    assert_eq!(rewritten.text, "ローカルで書き直したよ！");
    let sent = stub.take();
    assert_eq!(sent.len(), 2);
    assert!(sent[0].1.get("response_format").is_some() && sent[1].1.get("response_format").is_none());
    assert!(sent[1].1["messages"][0]["content"].as_str().unwrap().contains("JSON schema"));
    assert_eq!(sent[1].1["model"], "qwen3-8b");
    assert!(sent[1].0.get("authorization").is_none(), "no key, no header");

    // With schema support, one request does it.
    stub.supports_schema.store(true, Ordering::SeqCst);
    let _: RewriteDraftResponse = ctx.call("Assist", "RewriteDraft", &taro, &rewrite()).await.unwrap();
    assert_eq!(stub.take().len(), 1);

    // The local server counts like any other: no consent, no text.
    sqlx::query("UPDATE users SET birthdate = '2015-01-01' WHERE id = $1")
        .bind(hanako.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let direct = ctx.direct(&taro, &hanako).await;
    let message = ctx.text(&hanako, &direct.id, "ないしょ").await;
    let translate =
        TranslateMessageRequest { message_id: message.id.clone(), target_language: "en".into(), ..Default::default() };
    assert_eq!(ctx.reason("Assist", "TranslateMessage", &taro, &translate).await, "SENDER_DISALLOWS_AI");
    assert_eq!(stub.count(), 0);

    // The operator switches it off: the user's explicit choice is unavailable,
    // and the text does not silently go to another company.
    let off = UpdateAssistProviderRequest { enabled: false, ..compat(&base_url, true) };
    let _: UpdateAssistProviderResponse = ctx.call("AssistAdmin", "UpdateAssistProvider", &admin, &off).await.unwrap();
    assert_eq!(ctx.reason("Assist", "RewriteDraft", &taro, &rewrite()).await, "AI_PROVIDER_UNAVAILABLE");
    assert_eq!(stub.count(), 0);
}

#[tokio::test]
async fn the_allowance_covers_the_compatible_host_and_nothing_else() {
    let _turn = TURN.lock().await;
    let (stub, address) = start_stub().await;
    // Anthropic pointed at the same loopback stub, without the test loopback policy.
    let cfg = ChatConfig { anthropic_base_url: format!("http://{address}"), ..env_config() };
    let ctx = fresh(cfg).await;
    let admin = ctx.app.user_with_roles("admin", &["admin"]).await;
    let taro = ctx.app.user("taro").await;
    let _: UpdateAssistProviderResponse = ctx
        .call("AssistAdmin", "UpdateAssistProvider", &admin, &compat(&format!("http://{address}/v1"), true))
        .await
        .unwrap();

    // Anthropic (the default) is refused by the guard: the allowance is the compatible provider's alone.
    assert_eq!(ctx.reason("Assist", "RewriteDraft", &taro, &rewrite()).await, "AI_UNAVAILABLE");
    assert_eq!(stub.count(), 0, "nothing reached the loopback stub through Anthropic");

    // Link-local addresses (cloud metadata) stay refused, allowance or not.
    assert_eq!(
        ctx.reason("AssistAdmin", "UpdateAssistProvider", &admin, &compat("http://169.254.169.254/v1", true)).await,
        "BASE_URL_NOT_ALLOWED"
    );

    // Taking the allowance back closes the door again, for requests too.
    sqlx::query("UPDATE chat_ai_providers SET allow_private_host = false WHERE provider = 3")
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let choose = ChooseAssistProviderRequest {
        provider: AiProvider::AI_PROVIDER_OPENAI_COMPATIBLE.into(),
        ..Default::default()
    };
    let _: ChooseAssistProviderResponse = ctx.call("Assist", "ChooseAssistProvider", &taro, &choose).await.unwrap();
    assert_eq!(ctx.reason("Assist", "RewriteDraft", &taro, &rewrite()).await, "AI_UNAVAILABLE");
    assert_eq!(stub.count(), 0);

    // The default can be chosen among enabled providers, and unset again.
    let set =
        |provider: AiProvider| SetDefaultAssistProviderRequest { provider: provider.into(), ..Default::default() };
    let changed: SetDefaultAssistProviderResponse = ctx
        .call("AssistAdmin", "SetDefaultAssistProvider", &admin, &set(AiProvider::AI_PROVIDER_OPENAI_COMPATIBLE))
        .await
        .unwrap();
    let changed = changed.instance.into_option().unwrap();
    assert_eq!(changed.default_provider.as_known(), Some(AiProvider::AI_PROVIDER_OPENAI_COMPATIBLE));
    assert_eq!(changed.effective_default.as_known(), Some(AiProvider::AI_PROVIDER_OPENAI_COMPATIBLE));
    let unset: SetDefaultAssistProviderResponse = ctx
        .call("AssistAdmin", "SetDefaultAssistProvider", &admin, &set(AiProvider::AI_PROVIDER_UNSPECIFIED))
        .await
        .unwrap();
    assert_eq!(
        unset.instance.into_option().unwrap().effective_default.as_known(),
        Some(AiProvider::AI_PROVIDER_ANTHROPIC)
    );
}
