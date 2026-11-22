//! Per-token rate limits of the auth middleware against a real Valkey
//! (`TOKULY_VALKEY_URL`, as exported by `tools/dev/infra.sh env`).
use std::sync::Arc;

use async_trait::async_trait;
use tokuly_core::{
    api_tokens::{ApiCall, ApiToken, ApiTokenKind, ApiTokenVerifier, ApiTokens, RateTier, Scopes, TokenRejection},
    auth::{AuthLayer, Tokens, auth_middleware},
    cache::Cache,
    keys::Keys,
    proxies::TrustedProxies,
};
use tower::ServiceExt as _;
use uuid::Uuid;

/// Accepts one token, with the given limit, and remembers the reported calls.
struct OneToken {
    token: ApiToken,
    calls: parking_lot::Mutex<Vec<ApiCall>>,
}

#[async_trait]
impl ApiTokenVerifier for OneToken {
    async fn verify(&self, token: &str) -> Result<ApiToken, TokenRejection> {
        if token == "tkl_pat_limited" { Ok(self.token.clone()) } else { Err(TokenRejection::Invalid) }
    }

    async fn record(&self, _token: &ApiToken, call: ApiCall) {
        self.calls.lock().push(call);
    }
}

fn verifier(per_minute: u32) -> Arc<OneToken> {
    let token = ApiToken {
        kind: ApiTokenKind::Personal,
        // A fresh ID per test: the counter is keyed by it.
        id: Uuid::now_v7(),
        user_id: Uuid::now_v7(),
        app_id: None,
        scopes: Scopes::new(["notes:read"]),
        expires_at: None,
        rate: RateTier { name: "test".into(), per_minute },
    };
    Arc::new(OneToken { token, calls: Default::default() })
}

async fn app(verifier: Arc<OneToken>, enforce: bool) -> axum::Router {
    let url = std::env::var("TOKULY_VALKEY_URL").unwrap_or_else(|_| "redis://localhost:6379".to_owned());
    let cache = Cache::connect(&url).await.expect("Valkey is reachable");
    let registry = ApiTokens::default();
    registry.register(verifier);
    let tokens = Tokens::new(&Keys::new("0123456789abcdef0123456789abcdef")).unwrap();
    let layer =
        AuthLayer::new(tokens, TrustedProxies::parse(&[]).unwrap()).with_api_tokens(registry, enforce.then_some(cache));
    axum::Router::new().fallback(|| async { "ok" }).layer(axum::middleware::from_fn_with_state(layer, auth_middleware))
}

/// One call with the token: status, error reason and the `x-ratelimit-*` / `retry-after` headers.
async fn call(app: &axum::Router, path: &str) -> (u16, String, Vec<Option<u32>>) {
    let request = http::Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", "Bearer tkl_pat_limited")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let header = |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
    let number = |name: &str| header(name).and_then(|v| v.parse::<u32>().ok());
    let limits =
        ["x-ratelimit-limit", "x-ratelimit-remaining", "x-ratelimit-reset", "retry-after"].map(number).to_vec();
    (response.status().as_u16(), header("x-tokuly-error").unwrap_or_default(), limits)
}

const LIST_NOTES: &str = "/tokuly.notes.v1.NoteService/ListNotes";

#[tokio::test]
async fn a_token_is_held_to_its_requests_per_minute() {
    let verifier = verifier(3);
    let app = app(verifier.clone(), true).await;
    // Refused requests count too, so probing for scopes cannot be done for free.
    let (status, reason, limits) = call(&app, "/tokuly.notes.v1.NoteService/CreateNote").await;
    assert_eq!((status, reason.as_str()), (403, "INSUFFICIENT_SCOPE"));
    assert_eq!(limits[..2], [Some(3), Some(2)]);
    for remaining in [1, 0] {
        let (status, _, limits) = call(&app, LIST_NOTES).await;
        assert_eq!(status, 200);
        assert_eq!(limits[..2], [Some(3), Some(remaining)]);
        assert!(limits[2].is_some_and(|reset| (1..=60).contains(&reset)) && limits[3].is_none());
    }
    let (status, reason, limits) = call(&app, LIST_NOTES).await;
    assert_eq!((status, reason.as_str()), (429, "RATE_LIMITED"));
    assert_eq!(limits[..2], [Some(3), Some(0)]);
    assert_eq!(limits[3], limits[2], "retry-after says when the window ends");

    // The verifier hears about the refusal as well.
    for _ in 0..200 {
        if verifier.calls.lock().len() >= 4 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let errors: Vec<Option<String>> = verifier.calls.lock().iter().map(|call| call.error.clone()).collect();
    assert_eq!(errors.iter().filter(|e| e.as_deref() == Some("RATE_LIMITED")).count(), 1, "{errors:?}");
    assert_eq!(errors.iter().filter(|e| e.is_none()).count(), 2, "{errors:?}");
}

#[tokio::test]
async fn unlimited_tiers_and_deployments_without_limits_are_not_counted() {
    // A tier of 0 requests per minute means "no limit".
    let unlimited = app(verifier(0), true).await;
    // TOKULY_RATE_LIMITS=false: the layer has no limiter.
    let switched_off = app(verifier(1), false).await;
    for app in [&unlimited, &switched_off] {
        for _ in 0..3 {
            let (status, _, limits) = call(app, LIST_NOTES).await;
            assert_eq!(status, 200);
            assert_eq!(limits, [None, None, None, None]);
        }
    }
}
