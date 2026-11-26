//! A test binary removes what it created when it exits, even when its tests
//! panicked: this test runs a child copy of itself whose only test starts an
//! app and panics, then checks that nothing of the child's run is left.
use std::{collections::HashMap, process::Command};

use futures::TryStreamExt as _;
use sqlx::{Connection as _, PgConnection};
use tokuly_testkit::TestApp;

const CHILD: &str = "TOKULY_TESTKIT_AT_EXIT_CHILD";

#[tokio::test]
async fn a_panicking_test_binary_leaves_nothing_behind() {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_starts_an_app_and_panics", "--nocapture", "--test-threads", "1"])
        .env(CHILD, "1")
        .output()
        .expect("run the child");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!output.status.success(), "the child's test panics:\n{stdout}");
    // What the child's run created, as it printed it. libtest's own
    // "test … ..." may precede a print on the same line.
    let run: HashMap<&str, &str> =
        stdout.lines().filter_map(|line| line.split_once("child-run ")?.1.split_once('=')).collect();
    let database = run.get("database").copied().unwrap_or_default();
    assert!(database.starts_with("tokuly_test_"), "the child printed its run:\n{stdout}");

    let mut admin =
        PgConnection::connect(&std::env::var("TOKULY_DATABASE_URL").expect("TOKULY_DATABASE_URL")).await.unwrap();
    let database_left: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
        .bind(database)
        .fetch_one(&mut admin)
        .await
        .unwrap();
    assert!(!database_left, "{database} was dropped");

    let nats_prefix = run.get("nats_prefix").copied().unwrap_or_default();
    assert!(nats_prefix.starts_with('T'), "the child printed its NATS prefix:\n{stdout}");
    let js = async_nats::jetstream::new(
        async_nats::connect(std::env::var("TOKULY_NATS_URL").expect("TOKULY_NATS_URL")).await.unwrap(),
    );
    let streams_left: Vec<String> = js
        .stream_names()
        .try_collect::<Vec<String>>()
        .await
        .unwrap()
        .into_iter()
        .filter(|name| name.starts_with(nats_prefix))
        .collect();
    assert_eq!(streams_left, Vec::<String>::new(), "the streams of {nats_prefix} were deleted");
}

#[tokio::test]
async fn child_starts_an_app_and_panics() {
    if std::env::var(CHILD).is_err() {
        return;
    }
    let app = TestApp::start(&["0001", "0100", "0400"], |_, _, router| router).await;
    let cfg = &app.core.cfg;
    println!("child-run nats_prefix={}", cfg.nats_prefix);
    println!("child-run database={}", cfg.database_url.rsplit('/').next().unwrap().split('?').next().unwrap());
    panic!("the child's test fails");
}
