//! The AI services behind one trait: a request (system prompt, user text,
//! JSON schema of the answer) goes in, the answer comes out as the JSON
//! object the schema describes.
//!
//! - [`Anthropic`]: the Messages API (`POST {base}/v1/messages`), the
//!   answer constrained by `output_config.format`.
//! - [`OpenAi`]: the Chat Completions API (`POST {base}/chat/completions`),
//!   constrained by `response_format: json_schema`. The same type speaks to
//!   OpenAI-compatible servers (LM Studio…), which do not all support
//!   schemas: when such a server rejects `response_format`, the request is
//!   repeated without it, with the schema spelled out in the system prompt,
//!   and the answer is read as plain JSON.
//!
//! Every request goes through the shared outbound guard
//! ([`tokuly_core::egress`]): the base URLs are operator settings, so they
//! are checked like any address that comes from outside, with the one
//! exception the operator may grant to the compatible server's host.
use std::time::Duration;

use tokuly_core::egress::{self, HostAllowance, Policy};
use url::Url;

/// Room for the answer. Models may think before they write, and thinking
/// counts towards this limit, so it is far more than the answers need.
pub(crate) const MAX_TOKENS: u32 = 16000;
/// The most of an answer that is read.
const MAX_ANSWER_BYTES: usize = 1 << 20;
/// Beta that enables Anthropic's `fallbacks: "default"`.
pub(crate) const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

/// What the model is asked.
pub(crate) struct Task<'a> {
    pub system: &'a str,
    pub user: &'a str,
    /// The JSON schema of the answer (an object).
    pub schema: &'a serde_json::Value,
}

/// Why a request to the AI service did not produce an answer.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    /// Worth another attempt: the service is overloaded or unreachable.
    Transient(String),
    /// Will not get better by retrying (a rejected request, a wrong key).
    Permanent(String),
    /// The model declined the text.
    Refused,
}

/// One AI service, ready to be asked.
#[async_trait::async_trait]
pub(crate) trait Provider: Send + Sync {
    /// One attempt at `task`: the answer as the JSON value of the schema.
    async fn attempt(&self, task: &Task<'_>) -> Result<serde_json::Value, Failure>;
}

/// Where requests go and what they may reach.
pub(crate) struct Endpoint {
    pub url: Url,
    pub policy: Policy,
    /// The operator's exception for this endpoint's host, if granted.
    pub allowance: Option<HostAllowance>,
    pub timeout: Duration,
}

impl Endpoint {
    fn client(&self) -> Result<reqwest::Client, Failure> {
        let builder = match &self.allowance {
            Some(allowance) => egress::client_builder_allowing(self.policy, allowance.clone(), self.timeout),
            None => egress::client_builder(Some(self.policy), self.timeout),
        };
        builder.user_agent("Tokuly/1.0").build().map_err(|e| Failure::Permanent(format!("http client: {e}")))
    }

    /// Posts `body` and returns the status and (capped) answer.
    async fn post(
        &self,
        headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<(reqwest::StatusCode, bytes::Bytes), Failure> {
        egress::check_url_allowing(&self.url, self.policy, self.allowance.as_ref())
            .map_err(|refusal| Failure::Permanent(format!("the outbound guard refused the address: {refusal}")))?;
        let mut request = self.client()?.post(self.url.clone()).header("content-type", "application/json");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let mut response = request.json(body).send().await.map_err(|e| match egress::refusal_of(&e) {
            Some(refusal) => Failure::Permanent(format!("the outbound guard refused the address: {refusal}")),
            None => Failure::Transient(format!("request failed: {}", e.without_url())),
        })?;
        egress::check_peer_allowing(&response, self.policy, self.allowance.as_ref())
            .map_err(|refusal| Failure::Permanent(format!("the outbound guard refused the peer: {refusal}")))?;
        let status = response.status();
        let bytes = egress::read_capped(&mut response, MAX_ANSWER_BYTES)
            .await
            .map_err(|e| Failure::Transient(format!("answer could not be read: {}", e.without_url())))?;
        Ok((status, bytes))
    }
}

/// How a failed HTTP status is treated. The error type is logged, the
/// error message is not (it may quote user text). The type comes from any
/// server an operator points an OpenAI-compatible provider at, so only a
/// short identifier-like prefix of it is kept.
fn status_failure(status: reqwest::StatusCode, bytes: &[u8], kind_pointer: &str) -> Failure {
    let error_type = serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|json| json.pointer(kind_pointer).and_then(|kind| kind.as_str()).map(loggable_error_type))
        .unwrap_or_default();
    let description = format!("HTTP {status} {error_type}");
    if matches!(status.as_u16(), 408 | 409 | 429) || status.is_server_error() {
        Failure::Transient(description)
    } else {
        Failure::Permanent(description)
    }
}

/// Longest error type kept for a log line.
const MAX_ERROR_TYPE_CHARS: usize = 40;

/// An error type as a log line may carry it: ASCII letters, digits and
/// `_-.:` only (anything else becomes `?`), at most
/// [`MAX_ERROR_TYPE_CHARS`] characters, `...` marking a cut.
fn loggable_error_type(kind: &str) -> String {
    let mut out: String = kind
        .chars()
        .take(MAX_ERROR_TYPE_CHARS)
        .map(|c| if c.is_ascii_alphanumeric() || "_-.:".contains(c) { c } else { '?' })
        .collect();
    if kind.chars().nth(MAX_ERROR_TYPE_CHARS).is_some() {
        out.push_str("...");
    }
    out
}

fn decode(bytes: &[u8]) -> Result<serde_json::Value, Failure> {
    serde_json::from_slice(bytes).map_err(|e| Failure::Transient(format!("undecodable answer: {}", json_error(&e))))
}

/// What went wrong with a JSON value, without serde's message, which may
/// quote the value (model output derived from user text): the category
/// and the position only. For log lines.
pub(crate) fn json_error(error: &serde_json::Error) -> String {
    format!("{:?} error at line {} column {}", error.classify(), error.line(), error.column())
}

// ───────────────────────────── Anthropic ─────────────────────────────

/// Anthropic's Messages API.
pub(crate) struct Anthropic {
    pub endpoint: Endpoint,
    pub api_key: String,
    pub version: String,
    pub model: String,
    /// `output_config.effort`; empty omits it.
    pub effort: String,
    /// Send `fallbacks: "default"` (and its beta header).
    pub refusal_fallback: bool,
}

impl Anthropic {
    /// `{base}/v1/messages`.
    pub fn endpoint_url(base_url: &str) -> Result<Url, url::ParseError> {
        Url::parse(&format!("{}/v1/messages", base_url.trim_end_matches('/')))
    }

    /// The body of a Messages API request answering in JSON of the schema.
    pub fn request_body(&self, task: &Task<'_>) -> serde_json::Value {
        let mut output_config = serde_json::json!({ "format": { "type": "json_schema", "schema": task.schema } });
        if !self.effort.is_empty() {
            output_config["effort"] = serde_json::Value::String(self.effort.clone());
        }
        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "system": task.system,
            "messages": [{ "role": "user", "content": task.user }],
            "output_config": output_config,
        });
        if self.refusal_fallback {
            // A request the safety classifiers decline is retried by the API
            // on the fallback model Anthropic recommends for that case.
            body["fallbacks"] = serde_json::Value::String("default".to_owned());
        }
        body
    }
}

#[async_trait::async_trait]
impl Provider for Anthropic {
    async fn attempt(&self, task: &Task<'_>) -> Result<serde_json::Value, Failure> {
        let mut headers = vec![("x-api-key", self.api_key.as_str()), ("anthropic-version", self.version.as_str())];
        if self.refusal_fallback {
            headers.push(("anthropic-beta", FALLBACK_BETA));
        }
        let (status, bytes) = self.endpoint.post(&headers, &self.request_body(task)).await?;
        if !status.is_success() {
            return Err(status_failure(status, &bytes, "/error/type"));
        }
        parse_anthropic(&decode(&bytes)?)
    }
}

/// Extracts the structured answer from a Messages API response.
pub(crate) fn parse_anthropic(answer: &serde_json::Value) -> Result<serde_json::Value, Failure> {
    // A refusal is a successful response whose content must not be read.
    match answer.get("stop_reason").and_then(|reason| reason.as_str()) {
        Some("refusal") => return Err(Failure::Refused),
        Some("max_tokens") => return Err(Failure::Permanent("the answer was cut off".to_owned())),
        _ => {}
    }
    // Thinking and fallback blocks may surround the text.
    let text: String = answer
        .get("content")
        .and_then(|content| content.as_array())
        .map(|blocks| {
            blocks
                .iter()
                .filter(|block| block.get("type").and_then(|kind| kind.as_str()) == Some("text"))
                .filter_map(|block| block.get("text").and_then(|text| text.as_str()))
                .collect()
        })
        .unwrap_or_default();
    serde_json::from_str(text.trim())
        .map_err(|e| Failure::Permanent(format!("the answer is not the requested JSON: {}", json_error(&e))))
}

// ───────────────────────────── OpenAI ─────────────────────────────

/// OpenAI's Chat Completions API, or a server that speaks it.
pub(crate) struct OpenAi {
    pub endpoint: Endpoint,
    /// `None`: no `Authorization` header (a local server without keys).
    pub api_key: Option<String>,
    pub model: String,
    /// `reasoning_effort`; empty omits it.
    pub reasoning_effort: String,
    /// An OpenAI-compatible server rather than OpenAI itself: the older
    /// `max_tokens` parameter, no `reasoning_effort`, and the fallback to
    /// plain JSON when schemas are not supported.
    pub compatible: bool,
}

/// How the answer's shape is requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JsonMode {
    /// `response_format: json_schema` (strict).
    Schema,
    /// The schema spelled out in the system prompt, the answer read as
    /// plain JSON: for compatible servers without schema support.
    Prompted,
}

impl OpenAi {
    /// `{base}/chat/completions`.
    pub fn endpoint_url(base_url: &str) -> Result<Url, url::ParseError> {
        Url::parse(&format!("{}/chat/completions", base_url.trim_end_matches('/')))
    }

    pub fn request_body(&self, task: &Task<'_>, mode: JsonMode) -> serde_json::Value {
        let system = match mode {
            JsonMode::Schema => task.system.to_owned(),
            JsonMode::Prompted => format!(
                "{}\n\nAnswer with one JSON object that matches this JSON schema, and nothing else (no prose, no code fences):\n{}",
                task.system, task.schema
            ),
        };
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": task.user },
            ],
        });
        if mode == JsonMode::Schema {
            body["response_format"] = serde_json::json!({
                "type": "json_schema",
                "json_schema": { "name": "answer", "strict": true, "schema": task.schema },
            });
        }
        if self.compatible {
            body["max_tokens"] = MAX_TOKENS.into();
        } else {
            body["max_completion_tokens"] = MAX_TOKENS.into();
            if !self.reasoning_effort.is_empty() {
                body["reasoning_effort"] = serde_json::Value::String(self.reasoning_effort.clone());
            }
        }
        body
    }

    async fn send(&self, task: &Task<'_>, mode: JsonMode) -> Result<(reqwest::StatusCode, bytes::Bytes), Failure> {
        let authorization = self.api_key.as_ref().map(|key| format!("Bearer {key}"));
        let headers: Vec<(&str, &str)> =
            authorization.as_deref().map(|value| ("authorization", value)).into_iter().collect();
        self.endpoint.post(&headers, &self.request_body(task, mode)).await
    }
}

/// Whether a compatible server's answer says it does not understand
/// `response_format`: a client error that is not about the key, the model
/// or the rate.
fn rejects_schema(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 400 | 415 | 422 | 501)
}

#[async_trait::async_trait]
impl Provider for OpenAi {
    async fn attempt(&self, task: &Task<'_>) -> Result<serde_json::Value, Failure> {
        let (mut status, mut bytes) = self.send(task, JsonMode::Schema).await?;
        let mut mode = JsonMode::Schema;
        if self.compatible && rejects_schema(status) {
            tracing::info!(%status, "the compatible AI server rejected response_format; asking for plain JSON");
            (status, bytes) = self.send(task, JsonMode::Prompted).await?;
            mode = JsonMode::Prompted;
        }
        if !status.is_success() {
            return Err(status_failure(status, &bytes, "/error/type"));
        }
        // A server that ignored the schema still gets read leniently.
        parse_openai(&decode(&bytes)?, if self.compatible { JsonMode::Prompted } else { mode })
    }
}

/// Extracts the structured answer from a Chat Completions response.
pub(crate) fn parse_openai(answer: &serde_json::Value, mode: JsonMode) -> Result<serde_json::Value, Failure> {
    let choice =
        answer.pointer("/choices/0").ok_or_else(|| Failure::Permanent("the answer has no choice".to_owned()))?;
    let message = choice.get("message").cloned().unwrap_or_default();
    if message.get("refusal").and_then(|refusal| refusal.as_str()).is_some_and(|refusal| !refusal.is_empty()) {
        return Err(Failure::Refused);
    }
    match choice.get("finish_reason").and_then(|reason| reason.as_str()) {
        Some("content_filter") => return Err(Failure::Refused),
        Some("length") => return Err(Failure::Permanent("the answer was cut off".to_owned())),
        _ => {}
    }
    let text = message.get("content").and_then(|content| content.as_str()).unwrap_or_default().trim();
    let parsed = match mode {
        JsonMode::Schema => serde_json::from_str(text),
        JsonMode::Prompted => serde_json::from_str(text).or_else(|e| lenient_json(text).ok_or(e)),
    };
    parsed.map_err(|e| Failure::Permanent(format!("the answer is not the requested JSON: {}", json_error(&e))))
}

/// The JSON object inside an answer that wraps it in prose or code fences
/// (local models do), or a reasoning model's `<think>` section.
fn lenient_json(text: &str) -> Option<serde_json::Value> {
    let text = match text.rfind("</think>") {
        Some(end) => &text[end + "</think>".len()..],
        None => text,
    };
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end < start {
        return None;
    }
    serde_json::from_str::<serde_json::Value>(&text[start..=end]).ok().filter(|value| value.is_object())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(url: &str) -> Endpoint {
        Endpoint {
            url: Url::parse(url).unwrap(),
            policy: Policy::default(),
            allowance: None,
            timeout: Duration::from_secs(1),
        }
    }

    fn schema() -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": { "text": { "type": "string" } }, "required": ["text"], "additionalProperties": false })
    }

    fn anthropic(refusal_fallback: bool, effort: &str) -> Anthropic {
        Anthropic {
            endpoint: endpoint("https://api.anthropic.com/v1/messages"),
            api_key: "sk-test".into(),
            version: "2023-06-01".into(),
            model: "claude-opus-5-5".into(),
            effort: effort.into(),
            refusal_fallback,
        }
    }

    fn openai(compatible: bool) -> OpenAi {
        OpenAi {
            endpoint: endpoint("https://api.openai.com/v1/chat/completions"),
            api_key: Some("sk-test".into()),
            model: "gpt-5-mini".into(),
            reasoning_effort: "low".into(),
            compatible,
        }
    }

    #[test]
    fn endpoints_are_built_from_the_base_urls() {
        assert_eq!(Anthropic::endpoint_url("http://127.0.0.1:1/").unwrap().as_str(), "http://127.0.0.1:1/v1/messages");
        assert_eq!(
            OpenAi::endpoint_url("http://lmstudio.local:1234/v1/").unwrap().as_str(),
            "http://lmstudio.local:1234/v1/chat/completions"
        );
    }

    #[test]
    fn anthropic_requests_ask_for_json_and_carry_no_sampling_parameters() {
        let schema = schema();
        let task = Task { system: "system prompt", user: "user text", schema: &schema };
        let body = anthropic(true, "low").request_body(&task);
        assert_eq!(body["model"], "claude-opus-5-5");
        assert_eq!(body["system"], "system prompt");
        assert_eq!(body["messages"], serde_json::json!([{ "role": "user", "content": "user text" }]));
        assert_eq!(body["output_config"]["effort"], "low");
        assert_eq!(body["output_config"]["format"]["type"], "json_schema");
        assert_eq!(body["output_config"]["format"]["schema"]["required"], serde_json::json!(["text"]));
        assert_eq!(body["fallbacks"], "default");
        for removed in ["temperature", "top_p", "top_k", "thinking"] {
            assert!(body.get(removed).is_none(), "{removed} must not be sent");
        }
        let plain = anthropic(false, "").request_body(&task);
        assert!(plain.get("fallbacks").is_none());
        assert!(plain["output_config"].get("effort").is_none());
    }

    #[test]
    fn openai_requests_ask_for_a_strict_schema() {
        let schema = schema();
        let task = Task { system: "system prompt", user: "user text", schema: &schema };
        let body = openai(false).request_body(&task, JsonMode::Schema);
        assert_eq!(body["messages"][0], serde_json::json!({ "role": "system", "content": "system prompt" }));
        assert_eq!(body["messages"][1], serde_json::json!({ "role": "user", "content": "user text" }));
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(body["response_format"]["json_schema"]["strict"], true);
        assert_eq!(body["response_format"]["json_schema"]["schema"], schema);
        assert_eq!(body["reasoning_effort"], "low");
        assert!(body.get("max_completion_tokens").is_some() && body.get("max_tokens").is_none());
        for removed in ["temperature", "top_p"] {
            assert!(body.get(removed).is_none(), "{removed} must not be sent");
        }

        let compatible = openai(true).request_body(&task, JsonMode::Schema);
        assert!(compatible.get("max_tokens").is_some() && compatible.get("reasoning_effort").is_none());
        let prompted = openai(true).request_body(&task, JsonMode::Prompted);
        assert!(prompted.get("response_format").is_none());
        let system = prompted["messages"][0]["content"].as_str().unwrap();
        assert!(system.starts_with("system prompt") && system.contains("\"required\":[\"text\"]"));
        assert_eq!(prompted["messages"][1]["content"], "user text", "user text stays in its own turn");
    }

    #[test]
    fn anthropic_answers_are_read_from_the_text_blocks() {
        let answer = serde_json::json!({
            "stop_reason": "end_turn",
            "content": [{ "type": "thinking", "thinking": "" }, { "type": "text", "text": "{\"text\": \"おはよう！\"}" }],
        });
        assert_eq!(parse_anthropic(&answer).unwrap()["text"], "おはよう！");
        let refusal = serde_json::json!({ "stop_reason": "refusal", "content": [] });
        assert_eq!(parse_anthropic(&refusal).unwrap_err(), Failure::Refused);
        let cut = serde_json::json!({ "stop_reason": "max_tokens", "content": [{ "type": "text", "text": "{\"text\": \"お" }] });
        assert!(matches!(parse_anthropic(&cut), Err(Failure::Permanent(_))));
        let chatty = serde_json::json!({ "stop_reason": "end_turn", "content": [{ "type": "text", "text": "Sure! Here you go" }] });
        assert!(matches!(parse_anthropic(&chatty), Err(Failure::Permanent(_))));
        assert!(matches!(parse_anthropic(&serde_json::json!({})), Err(Failure::Permanent(_))));
    }

    #[test]
    fn openai_answers_refusals_and_cut_offs_are_told_apart() {
        let answer = |content: &str, finish: &str| serde_json::json!({ "choices": [{ "message": { "role": "assistant", "content": content, "refusal": null }, "finish_reason": finish }] });
        assert_eq!(parse_openai(&answer("{\"text\":\"やあ\"}", "stop"), JsonMode::Schema).unwrap()["text"], "やあ");
        let refused = serde_json::json!({ "choices": [{ "message": { "content": null, "refusal": "I can't help with that." }, "finish_reason": "stop" }] });
        assert_eq!(parse_openai(&refused, JsonMode::Schema).unwrap_err(), Failure::Refused);
        assert_eq!(parse_openai(&answer("", "content_filter"), JsonMode::Schema).unwrap_err(), Failure::Refused);
        assert!(matches!(parse_openai(&answer("{\"te", "length"), JsonMode::Schema), Err(Failure::Permanent(_))));
        assert!(matches!(
            parse_openai(&serde_json::json!({ "choices": [] }), JsonMode::Schema),
            Err(Failure::Permanent(_))
        ));

        // Plain JSON from a local model: fences, prose and thinking around it.
        let fenced = answer("Here it is:\n```json\n{\"text\": \"やあ\"}\n```", "stop");
        assert!(
            matches!(parse_openai(&fenced, JsonMode::Schema), Err(Failure::Permanent(_))),
            "OpenAI itself is strict"
        );
        assert_eq!(parse_openai(&fenced, JsonMode::Prompted).unwrap()["text"], "やあ");
        let thinking = answer("<think>the user wants {braces}</think>\n{\"text\": \"やあ\"}", "stop");
        assert_eq!(parse_openai(&thinking, JsonMode::Prompted).unwrap()["text"], "やあ");
        assert!(matches!(
            parse_openai(&answer("no json here", "stop"), JsonMode::Prompted),
            Err(Failure::Permanent(_))
        ));
    }

    #[test]
    fn logged_error_types_are_short_and_ascii() {
        let failure = |body: serde_json::Value| {
            status_failure(reqwest::StatusCode::BAD_REQUEST, body.to_string().as_bytes(), "/error/type")
        };
        let description = |failure| match failure {
            Failure::Permanent(description) => description,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            description(failure(serde_json::json!({ "error": { "type": "invalid_request_error" } }))),
            "HTTP 400 Bad Request invalid_request_error"
        );
        let hostile = format!("bad\n{{\"x\": \"秘密\"}} {}", "a".repeat(500));
        let logged = description(failure(serde_json::json!({ "error": { "type": hostile } })));
        let error_type = logged.strip_prefix("HTTP 400 Bad Request ").unwrap();
        assert_eq!(error_type, format!("bad???x?:???????{}...", "a".repeat(24)));
        assert!(error_type.is_ascii() && !error_type.contains('\n'));
    }
}
