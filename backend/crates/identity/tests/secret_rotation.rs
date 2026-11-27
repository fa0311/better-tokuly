//! Replacing `TOKULY_SECRET` (docs/configuration.md): with the old one as
//! `TOKULY_SECRET_PREVIOUS` nobody is signed out, `tokuly rekey` carries
//! what is sealed over, and once the old one is removed it opens nothing.
use tokuly_core::{
    auth::ClientMeta,
    keys::{Keys, b64url, b64url_decode},
};
use tokuly_identity::{sessions::Issued, users};
use tokuly_proto::identity::*;
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

async fn rekey(app: &TestApp) {
    for column in tokuly_identity::SEALED {
        tokuly_core::rekey::rekey(&app.core.db, &app.core.keys, column).await.unwrap();
    }
}

/// Turns two-step sign-in on for `user`; returns the recovery codes.
async fn two_step(app: &TestApp, user: &TestUser) -> Vec<String> {
    let setup: BeginTotpSetupResponse = app
        .call("tokuly.identity.v1.AccountService/BeginTotpSetup", Some(user), &BeginTotpSetupRequest::default())
        .await
        .unwrap();
    let code = totp_now(&setup.secret);
    let enabled: EnableTotpResponse = app
        .call(
            "tokuly.identity.v1.AccountService/EnableTotp",
            Some(user),
            &EnableTotpRequest { code, ..Default::default() },
        )
        .await
        .unwrap();
    enabled.recovery_codes
}

/// Turns two-step sign-in off with a recovery code; the error's reason if the code is refused.
async fn recover(app: &TestApp, user: &TestUser, code: &str) -> Result<(), String> {
    let user = app.sign_in(user.id).await;
    let request = DisableTotpRequest { code: code.to_owned(), ..Default::default() };
    app.call::<_, DisableTotpResponse>("tokuly.identity.v1.AccountService/DisableTotp", Some(&user), &request)
        .await
        .map(|_| ())
        .map_err(|e| e.reason)
}

fn totp_now(secret_base32: &str) -> String {
    use totp_rs::{Builder, Secret};
    let now = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    // Not at the very end of a step: the server might be in the next one.
    if now() % 30 >= 27 {
        std::thread::sleep(std::time::Duration::from_secs(4));
    }
    let totp = Builder::new()
        .with_secret(Secret::try_from_base32(secret_base32).unwrap())
        .with_account_name("test".to_owned())
        .with_issuer(Some("Tokuly"))
        .build()
        .unwrap();
    totp.generate(now()).to_string()
}

const NEWER: &str = "rotation-newer-secret-rotation-newer-secret";

// One test: `forget_recovery_codes` forgets the codes of every account there is.
#[tokio::test]
async fn recovery_codes_outlive_a_routine_change_of_the_secret_but_not_one_for_a_leak() {
    let before = app(OLD, "").await;
    let user = before.user("recovery").await;
    let codes = two_step(&before, &user).await;
    rekey(&app(NEW, OLD).await).await;
    let after = app(NEW, "").await;
    let carried = recover(&after, &user, &codes[0]).await;
    assert_eq!(carried, Ok(()), "a code made before the change, after the old secret is gone");

    // (Another account: a code of this time step is spent for the first.)
    let user = after.user("leaked").await;
    let codes = two_step(&after, &user).await;
    let (kept, forgotten) = (after.user("hashed-kept").await, after.user("hashed-forgotten").await);
    for account in [&kept, &forgotten] {
        two_step(&after, account).await;
        hashed_codes(&after, account, NEW, &["aaaa-1111", "bbbb-2222"]).await;
    }
    let during = app(NEWER, NEW).await;
    assert_eq!(recover(&during, &kept, "AAAA-1111").await, Ok(()), "a hashed code, under the previous secret");
    assert!(during.identity.forget_recovery_codes().await.unwrap() >= 2);
    assert_eq!(
        recover(&during, &forgotten, "aaaa-1111").await.unwrap_err(),
        "INVALID_TOTP",
        "hashed codes are forgotten too, though the secret they were made under is still accepted"
    );
    rekey(&during).await;
    let after = app(NEWER, "").await;
    assert_eq!(recover(&after, &user, &codes[0]).await.unwrap_err(), "INVALID_TOTP");
    let noted: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM security_events WHERE user_id = $1 AND type = 'RECOVERY_CODES_REVOKED')",
    )
    .bind(user.id)
    .fetch_one(&after.core.db)
    .await
    .unwrap();
    assert!(noted, "the account's security log says why its codes stopped working");
}

/// Gives `user` recovery codes as versions before sealing stored them: keyed hashes under `secret`.
async fn hashed_codes(app: &TestApp, user: &TestUser, secret: &str, codes: &[&str]) {
    let keys = Keys::new(secret);
    let hashes: Vec<String> = codes.iter().map(|code| keys.token_hash(&format!("recovery:{code}"))).collect();
    sqlx::query("UPDATE users SET recovery_codes = NULL, recovery_code_hashes = $2 WHERE id = $1")
        .bind(user.id)
        .bind(hashes)
        .execute(&app.core.db)
        .await
        .unwrap();
}

/// How many values of the scratch table only `OLD` opens.
async fn left_under_the_old_secret(app: &TestApp) -> usize {
    let stored: Vec<Vec<u8>> = sqlx::query_scalar("SELECT sealed FROM rekey_under_writes").fetch_all(&app.core.db).await.unwrap();
    stored.iter().filter(|sealed| Keys::new(NEW).open("scratch", sealed).is_none()).count()
}

#[tokio::test]
async fn rekey_leaves_nothing_behind_in_rows_the_servers_write_meanwhile() {
    const ROWS: i32 = 3000;
    let during = app(NEW, OLD).await;
    let db = during.core.db.clone();
    sqlx::query("CREATE TABLE IF NOT EXISTS rekey_under_writes (id int PRIMARY KEY, sealed bytea NOT NULL, touched int NOT NULL DEFAULT 0)")
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("TRUNCATE rekey_under_writes").execute(&db).await.unwrap();
    let old = Keys::new(OLD);
    let sealed: Vec<Vec<u8>> = (0..ROWS).map(|id| old.seal("scratch", id.to_string().as_bytes())).collect();
    sqlx::query("INSERT INTO rekey_under_writes (id, sealed) SELECT * FROM unnest($1::int[], $2::bytea[])")
        .bind((0..ROWS).collect::<Vec<_>>())
        .bind(&sealed)
        .execute(&db)
        .await
        .unwrap();

    // A server at work: other columns of the same rows change all the time.
    let writer = tokio::spawn({
        let db = db.clone();
        async move {
            loop {
                sqlx::query("UPDATE rekey_under_writes SET touched = touched + 1 WHERE id = floor(random() * $1)::int")
                    .bind(ROWS)
                    .execute(&db)
                    .await
                    .unwrap();
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        }
    });
    let column = tokuly_core::rekey::Sealed::bytes("rekey_under_writes", "sealed", "scratch");
    let done = tokuly_core::rekey::rekey(&db, &during.core.keys, &column).await;
    writer.abort();
    let _ = writer.await;

    assert_eq!(done.unwrap().resealed, ROWS as u64);
    assert_eq!(left_under_the_old_secret(&during).await, 0, "removing the old secret would make these unreadable");
    let opened: Vec<Vec<u8>> = sqlx::query_scalar("SELECT sealed FROM rekey_under_writes ORDER BY id").fetch_all(&db).await.unwrap();
    assert!(opened.iter().enumerate().all(|(id, sealed)| Keys::new(NEW).open("scratch", sealed) == Some(id.to_string().into_bytes())));
}
