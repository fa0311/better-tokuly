//! API tokens and client secrets across a change of `TOKULY_SECRET`: only
//! their keyed hashes are stored, so each is stored again under the new
//! secret when it is used while the previous one is still set.
use tokuly_proto::dev::*;

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

#[tokio::test]
async fn a_token_revoked_after_its_first_use_under_the_new_secret_stops_everywhere_at_once() {
    // A server not restarted yet keeps the token's answer under its old hash.
    let before = start_with_secret(OLD, "").await;
    let alice = before.app.user("alice").await;
    let (token, secret) = personal_token(&before, &alice, &["identity:read"]).await;
    assert!(whoami(&before, &with_token(&alice, &secret)).await.is_ok());

    let during = start_with_secret(NEW, OLD).await;
    assert!(whoami(&during, &with_token(&alice, &secret)).await.is_ok(), "stored again under the new hash");
    let revoke = RevokeTokenRequest { token_id: token.id, ..Default::default() };
    during
        .app
        .call::<_, RevokeTokenResponse>(
            &format!("{TOKEN_SERVICE}/RevokeToken"),
            Some(&during.app.sign_in(alice.id).await),
            &revoke,
        )
        .await
        .unwrap();
    assert!(whoami(&during, &with_token(&alice, &secret)).await.is_err());
    assert!(whoami(&before, &with_token(&alice, &secret)).await.is_err(), "nor under the old hash it was cached by");
}
