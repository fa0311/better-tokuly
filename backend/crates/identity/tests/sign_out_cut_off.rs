//! Where `tokuly sessions revoke-all` cuts, and that the cut holds while it
//! walks: the instant comes from the database's clock, and a Valkey that
//! lost it meanwhile learns it again with the next batch. A test binary of
//! its own: signing everyone out reaches every account of the database.
use std::time::Duration;

use tokuly_core::{SESSIONS_CUT_OFF_MARGIN, auth::AuthState, time::Time};
use tokuly_testkit::TestApp;
use uuid::Uuid;

const MIGRATIONS: &[&str] = &["0001", "0100", "0400"];

async fn app() -> TestApp {
    TestApp::start(MIGRATIONS, |_core, identity, router| identity.register(router)).await
}

async fn accepted(server: &TestApp, access_token: &str) -> bool {
    match server.core.tokens.authenticate_shared(access_token).await {
        AuthState::Authenticated(_) => true,
        AuthState::Invalid("SESSION_REVOKED") => false,
        state => panic!("{state:?}"),
    }
}

/// The session at `offset` in the order the walk takes them.
async fn walked_at(app: &TestApp, offset: i64) -> Uuid {
    sqlx::query_scalar("SELECT id FROM sessions WHERE revoked_at IS NULL ORDER BY id OFFSET $1 LIMIT 1")
        .bind(offset)
        .fetch_one(&app.core.db)
        .await
        .unwrap()
}

/// Waits until the walk waits for a held row, `walked` revoked before it.
async fn walk_waits_after(app: &TestApp, walked: Uuid) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let (waiting, revoked): (bool, bool) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity
                            WHERE datname = current_database() AND wait_event_type = 'Lock' AND query LIKE '%WITH batch AS%'),
                    (SELECT revoked_at IS NOT NULL FROM sessions WHERE id = $1)",
        )
        .bind(walked)
        .fetch_one(&app.core.db)
        .await
        .unwrap();
        if waiting && revoked {
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "the walk did not get there");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn the_cut_off_is_the_databases_and_survives_valkey_losing_it_mid_run() {
    let serving = app().await;
    let alice = serving.user("alice").await;
    // Three statements' worth.
    sqlx::query(
        "INSERT INTO sessions (user_id, refresh_hash, refresh_sealed, expires_at)
         SELECT $1, 'bulk-' || gen_random_uuid(), '\\x00', now() + interval '30 days' FROM generate_series(1, 1200)",
    )
    .bind(alice.id)
    .execute(&serving.core.db)
    .await
    .unwrap();
    // The newest, so the last batch reaches it.
    let mallory = serving.user("mallory").await;

    // Not the operator's clock: one behind the database's would leave the
    // sessions of the last moments out of both the cut-off and the walk.
    let cut_off = serving.core.sessions_cut_off().await.unwrap();
    let (ahead, newest): (f64, Time) =
        sqlx::query_as("SELECT extract(epoch FROM $1 - now())::float8, max(created_at) FROM sessions")
            .bind(cut_off)
            .fetch_one(&serving.core.db)
            .await
            .unwrap();
    let margin = SESSIONS_CUT_OFF_MARGIN.as_seconds_f64();
    assert!(ahead > margin - 1.0 && ahead <= margin, "the database's now plus the margin, not {ahead}s ahead");
    assert!(newest <= cut_off, "the session made last is in");

    // The walk is held in the second batch, then in the third.
    let (second, third) = (walked_at(&serving, 700).await, walked_at(&serving, 1100).await);
    let (last_of_first, last_of_second) = (walked_at(&serving, 499).await, walked_at(&serving, 999).await);
    let hold = |id: Uuid| {
        let db = serving.core.db.clone();
        async move {
            let mut held = db.begin().await.unwrap();
            sqlx::query("SELECT 1 FROM sessions WHERE id = $1 FOR UPDATE").bind(id).execute(&mut *held).await.unwrap();
            held
        }
    };
    let (in_second, in_third) = (hold(second).await, hold(third).await);

    let valkey = &serving.core.cache;
    let key = serving.core.tokens.cut_off_key().to_owned();
    let restarted_valkey_meanwhile = async {
        walk_waits_after(&serving, last_of_first).await;
        // Valkey restarts without persistence: the cut-off is gone.
        valkey.del(&key).await.unwrap();
        in_second.commit().await.unwrap();
        walk_waits_after(&serving, last_of_second).await;
        assert!(valkey.exists(&key).await.unwrap(), "written again after the batch");
        // A server started now knows only what Valkey has.
        let restarted = app().await;
        assert!(!accepted(&restarted, &mallory.token).await, "a session not walked yet is refused");
        in_third.commit().await.unwrap();
    };
    let (revoked, ()) = tokio::join!(serving.identity.sign_everyone_out(cut_off), restarted_valkey_meanwhile);
    assert!(revoked.unwrap() >= 1200 + 2);
    assert!(!accepted(&app().await, &mallory.token).await);
}
