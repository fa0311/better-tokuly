//! API tokens and client secrets across a change of `TOKULY_SECRET`: only
//! their keyed hashes are stored, so each is stored again under the new
//! secret when it is used while the previous one is still set.
use crate::common::*;

const OLD: &str = "rotation-old-secret-rotation-old-secret";
const NEW: &str = "rotation-new-secret-rotation-new-secret";

#[tokio::test]
async fn an_api_token_used_during_the_change_outlives_the_previous_secret() {
    let before = start_with_secret(OLD, "").await;
    let alice = before.app.user("alice").await;
    // A password, so that the token also carries a mark of it (keyed by the secret too).
    sqlx::query("UPDATE users SET password_hash = 'an-argon2-hash' WHERE id = $1")
        .bind(alice.id)
        .execute(&before.app.core.db)
        .await
        .unwrap();
    let (_, used) = personal_token(&before, &alice, &["identity:read"]).await;
    let (_, unused) = personal_token(&before, &alice, &["identity:read"]).await;

    let during = start_with_secret(NEW, OLD).await;
    for _ in 0..2 {
        assert!(whoami(&during, &with_token(&alice, &used)).await.is_ok(), "found under the old hash, then the new");
    }

    let after = start_with_secret(NEW, "").await;
    assert!(whoami(&after, &with_token(&alice, &used)).await.is_ok());
    assert!(whoami(&after, &with_token(&alice, &unused)).await.is_err(), "never used while both were set");
}
