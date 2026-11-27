//! What the procedure for a leaked secret does to every account
//! (docs/configuration.md): `tokuly sessions revoke-all` signs everyone out
//! on every server at once, and `rekey --forget-recovery-codes` tells each
//! account by mail. A test binary of its own: both reach every account of
//! the database.
use std::time::Duration;

use chrono::Utc;
use tokuly_core::{
    auth::{AuthState, ClientMeta},
    events::Event,
};
use tokuly_identity::{maintenance::RECOVERY_CODES_REVOKED, sessions::Issued, users};
use tokuly_testkit::{TestApp, TestUser};
use uuid::Uuid;

const MIGRATIONS: &[&str] = &["0001", "0100", "0400"];

async fn app() -> TestApp {
    TestApp::start(MIGRATIONS, |_core, identity, router| identity.register(router)).await
}

async fn session(app: &TestApp, user: &TestUser) -> Issued {
    let row = users::get(&app.core, user.id).await.unwrap();
    app.identity.create_session(&row, None, &ClientMeta::default()).await.unwrap()
}

async fn refresh(app: &TestApp, refresh_token: &str) -> Result<Issued, String> {
    app.identity
        .refresh_session(refresh_token, &ClientMeta::default())
        .await
        .map(|(issued, _)| issued)
        .map_err(|e| e.to_string())
}

async fn accepted(server: &TestApp, access_token: &str) -> bool {
    match server.core.tokens.authenticate_shared(access_token).await {
        AuthState::Authenticated(_) => true,
        AuthState::Invalid("SESSION_REVOKED") => false,
        state => panic!("{state:?}"),
    }
}

// One test: signing everyone out reaches every session of the database.
#[tokio::test]
async fn everyone_signs_in_again_and_every_server_refuses_the_old_tokens_at_once() {
    let (serving, elsewhere) = (app().await, app().await);
    let alice = serving.user("alice").await;
    let idle = session(&serving, &serving.user("idle").await).await;
    sqlx::query("UPDATE sessions SET last_active_at = now() - interval '2 hours' WHERE id = $1")
        .bind(idle.session_id)
        .execute(&serving.core.db)
        .await
        .unwrap();
    // More than one statement's worth, so that the batches are walked through.
    sqlx::query(
        "INSERT INTO sessions (user_id, refresh_hash, refresh_sealed, expires_at)
         SELECT $1, 'bulk-' || gen_random_uuid(), '\\x00', now() + interval '30 days' FROM generate_series(1, 1200)",
    )
    .bind(alice.id)
    .execute(&serving.core.db)
    .await
    .unwrap();
    // Devices that refresh while it runs.
    let refreshing: Vec<Issued> = {
        let bob = serving.user("bob").await;
        let mut sessions = Vec::new();
        for _ in 0..4 {
            sessions.push(session(&serving, &bob).await);
        }
        sessions
    };
    assert!(accepted(&elsewhere, &alice.token).await);

    let started = Utc::now();
    let kept = serving.user("signed-in-meanwhile").await;
    let refreshes = refreshing.into_iter().map(|first| {
        let serving = &serving;
        async move {
            let (mut issued, mut tokens) = (first.refresh_token, vec![first.access_token]);
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            while let Ok(next) = refresh(serving, &issued).await
                && tokio::time::Instant::now() < deadline
            {
                tokens.push(next.access_token);
                issued = next.refresh_token;
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            tokens
        }
    });
    let (revoked, issued) =
        tokio::join!(serving.identity.sign_everyone_out(started), futures::future::join_all(refreshes));
    assert!(revoked.unwrap() >= 1200 + 2 + 4);

    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions WHERE revoked_at IS NULL AND created_at <= $1")
        .bind(started)
        .fetch_one(&serving.core.db)
        .await
        .unwrap();
    assert_eq!(left, 0, "every session made before signs in again");
    assert!(refresh(&serving, &idle.refresh_token).await.is_err(), "nor can a session nobody used lately refresh");
    assert!(!accepted(&elsewhere, &alice.token).await, "another server refuses the access token on its next request");
    for token in issued.iter().flatten() {
        assert!(!accepted(&elsewhere, token).await, "nor one a refresh handed out while it ran");
    }
    assert!(accepted(&elsewhere, &kept.token).await, "who signed in meanwhile stays signed in");
    // Connections already open (the realtime gateway) hear of it.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !elsewhere.core.tokens.is_revoked(alice.session_id) {
        assert!(tokio::time::Instant::now() < deadline, "the revocation was not broadcast");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The event [`forget_recovery_codes`](tokuly_identity::Identity::forget_recovery_codes) left for `user`.
async fn told(app: &TestApp, user: Uuid) -> Vec<Event> {
    let user = serde_json::json!({ "user_id": user });
    app.events(RECOVERY_CODES_REVOKED).await.into_iter().filter(|event| event.data == user).collect()
}

// One test: `forget_recovery_codes` forgets the codes of every account there is.
#[tokio::test]
async fn everyone_whose_recovery_codes_were_forgotten_gets_one_mail() {
    let app = app().await;
    let (coded, uncoded, addressless) =
        (app.user("coded").await, app.user("uncoded").await, app.user("addressless").await);
    for user in [&coded, &addressless] {
        sqlx::query("UPDATE users SET recovery_code_hashes = '{a-hash}' WHERE id = $1")
            .bind(user.id)
            .execute(&app.core.db)
            .await
            .unwrap();
    }
    sqlx::query("UPDATE users SET email = NULL WHERE id = $1")
        .bind(addressless.id)
        .execute(&app.core.db)
        .await
        .unwrap();

    assert!(app.identity.forget_recovery_codes().await.unwrap() >= 2);
    let event = match told(&app, coded.id).await.as_slice() {
        [event] => event.clone(),
        other => panic!("one event per account, on the outbox with the change: {other:?}"),
    };
    assert!(told(&app, uncoded.id).await.is_empty(), "an account without codes lost nothing");

    // The mail server does not answer: nothing is noted, the next delivery sends it.
    let unreachable = TestApp::start_with(
        MIGRATIONS,
        |cfg| cfg.smtp_url = "smtp://127.0.0.1:9".to_owned(),
        |_, identity, router| identity.register(router),
    )
    .await;
    assert!(unreachable.identity.tell_recovery_codes_revoked(&event).await.is_err());

    // Delivered twice at once, and once more later: one mail.
    let (first, second) = tokio::join!(
        app.identity.tell_recovery_codes_revoked(&event),
        app.identity.tell_recovery_codes_revoked(&event)
    );
    let sent = [&first, &second].iter().filter(|done| matches!(done, Ok(true))).count();
    assert_eq!(sent, 1, "{first:?} {second:?}");
    assert!(!app.identity.tell_recovery_codes_revoked(&event).await.unwrap(), "sent already");

    let [event] = told(&app, addressless.id).await.try_into().unwrap();
    assert!(!app.identity.tell_recovery_codes_revoked(&event).await.unwrap(), "no address to tell");
}
