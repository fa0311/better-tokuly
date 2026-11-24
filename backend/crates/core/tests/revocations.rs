//! Session revocations shared through Valkey (`TOKULY_VALKEY_URL`, as
//! exported by `tools/dev/infra.sh env`): a session revoked by one process
//! is refused by every other one on its next request, without waiting for
//! the access token to expire or for a broadcast to arrive.
use tokuly_core::{
    auth::{AuthState, Tokens},
    cache::Cache,
    keys::Keys,
};
use uuid::Uuid;

const SECRET: &str = "0123456789abcdef0123456789abcdef";

/// A process's token verifier: same secret, same Valkey, nothing else shared.
async fn process() -> Tokens {
    let url = std::env::var("TOKULY_VALKEY_URL").unwrap_or_else(|_| "redis://localhost:6379".to_owned());
    let cache = Cache::connect(&url).await.expect("Valkey is reachable");
    Tokens::new(&Keys::new(SECRET)).unwrap().with_revocations(cache)
}

#[tokio::test]
async fn a_revoked_session_is_refused_by_another_process_on_its_next_request() {
    let (identity, api) = (process().await, process().await);
    let (user, session, other) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    let (token, _) = identity.issue(user, session, &[]).unwrap();
    let (other_token, _) = identity.issue(user, other, &[]).unwrap();
    assert!(
        matches!(api.authenticate_shared(&token).await, AuthState::Authenticated(ctx) if ctx.session_id == session)
    );

    // No broadcast reaches `api`: only Valkey knows.
    identity.revoke(session).await;
    assert!(matches!(api.authenticate_shared(&token).await, AuthState::Invalid("SESSION_REVOKED")));
    assert!(
        matches!(api.authenticate_shared(&other_token).await, AuthState::Authenticated(_)),
        "other sessions are untouched"
    );
    // Once seen, the revocation is also known in memory (for synchronous checks).
    assert_eq!(api.verify(&token).unwrap_err(), "SESSION_REVOKED");
}

#[tokio::test]
async fn a_process_that_starts_later_knows_earlier_revocations() {
    let identity = process().await;
    let session = Uuid::now_v7();
    let (token, _) = identity.issue(Uuid::now_v7(), session, &[]).unwrap();
    identity.revoke(session).await;

    let gateway = process().await;
    assert!(gateway.verify(&token).is_ok(), "nothing loaded yet");
    assert!(gateway.load_revocations().await.unwrap() >= 1);
    assert_eq!(gateway.verify(&token).unwrap_err(), "SESSION_REVOKED");
}
