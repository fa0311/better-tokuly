//! Replacing `TOKULY_SECRET` (docs/configuration.md): with the old one as
//! `TOKULY_SECRET_PREVIOUS` nobody is signed out, `tokuly rekey` carries
//! what is sealed over, and once the old one is removed it opens nothing.
use tokuly_core::{
    auth::ClientMeta,
    keys::{Keys, b64url, b64url_decode},
};
use tokuly_identity::{sessions::Issued, users};
use tokuly_testkit::{TestApp, TestUser};

const MIGRATIONS: &[&str] = &["0001", "0100", "0400"];
const OLD: &str = "rotation-old-secret-rotation-old-secret";
const NEW: &str = "rotation-new-secret-rotation-new-secret";

async fn app(secret: &str, previous: &str) -> TestApp {
    TestApp::start_with(
        MIGRATIONS,
        |cfg| {
            cfg.secret = secret.to_owned();
            cfg.secret_previous = previous.to_owned();
        },
        |_core, identity, router| identity.register(router),
    )
    .await
}

async fn session(app: &TestApp, user: &TestUser) -> Issued {
    let row = users::get(&app.core, user.id).await.unwrap();
    app.identity.create_session(&row, None, &ClientMeta::default()).await.unwrap()
}

async fn refresh(app: &TestApp, issued: &Issued) -> Result<Issued, String> {
    app.identity.refresh_session(&issued.refresh_token, &ClientMeta::default()).await.map(|(issued, _)| issued).map_err(|e| e.to_string())
}

#[tokio::test]
async fn nobody_is_signed_out_while_the_previous_secret_is_set() {
    let before = app(OLD, "").await;
    let user = before.user("rotation").await;
    let issued = session(&before, &user).await;

    let during = app(NEW, OLD).await;
    assert_eq!(during.core.tokens.verify(&user.token).unwrap().user_id, user.id, "access tokens of the old secret");
    let refreshed = refresh(&during, &issued).await.expect("a refresh token of the old secret");

    let after = app(NEW, "").await;
    assert!(after.core.tokens.verify(&user.token).is_err(), "the old secret signs nothing any more");
    refresh(&after, &refreshed).await.expect("what the change issued is under the new secret");
}

#[tokio::test]
async fn rekey_carries_over_what_nobody_used_and_without_it_the_old_secret_opens_nothing() {
    let before = app(OLD, "").await;
    let user = before.user("rekey").await;
    let (forgotten, kept) = (session(&before, &user).await, session(&before, &user).await);
    let seed = b64url(&Keys::new(OLD).seal("totp", b"JBSWY3DPEHPK3PXP"));
    sqlx::query("UPDATE users SET totp_secret = $2 WHERE id = $1").bind(user.id).bind(&seed).execute(&before.core.db).await.unwrap();

    let after = app(NEW, "").await;
    assert!(refresh(&after, &forgotten).await.is_err(), "the old secret removed before rekey");

    let during = app(NEW, OLD).await;
    for column in tokuly_identity::SEALED {
        tokuly_core::rekey::rekey(&during.core.db, &during.core.keys, column).await.unwrap();
    }
    refresh(&after, &kept).await.expect("rekey wrote the session's hash under the new secret");
    let stored: String =
        sqlx::query_scalar("SELECT totp_secret FROM users WHERE id = $1").bind(user.id).fetch_one(&after.core.db).await.unwrap();
    let opened = Keys::new(NEW).open("totp", &b64url_decode(&stored).unwrap());
    assert_eq!(opened.as_deref(), Some(b"JBSWY3DPEHPK3PXP".as_slice()), "the TOTP seed sealed again");

    let again = tokuly_core::rekey::rekey(&during.core.db, &during.core.keys, &tokuly_identity::SEALED[0]).await.unwrap();
    assert_eq!(again.resealed, 0, "a second run has nothing left to do");
}
