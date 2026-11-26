//! A test binary removes what it created when it exits, even when its tests
//! panicked: this test runs a child copy of itself whose only test starts an
//! app and panics, then checks that nothing of the child's run is left.
use std::{collections::HashMap, process::Command};

use bytes::Bytes;
use clap::Parser;
use futures::TryStreamExt as _;
use sqlx::{Connection as _, PgConnection};
use tokuly_core::{config::Config, storage::Bucket};
use tokuly_testkit::TestApp;

const CHILD: &str = "TOKULY_TESTKIT_AT_EXIT_CHILD";

#[derive(Parser)]
struct EnvConfig {
    #[command(flatten)]
    core: Config,
}

fn config() -> Config {
    EnvConfig::parse_from(["at-exit", "--secret", "test-secret-test-secret-test-secret-test"]).core
}

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
    let printed: HashMap<&str, &str> =
        stdout.lines().filter_map(|line| line.split_once("child-run ")?.1.split_once('=')).collect();
    let child = |key: &str| *printed.get(key).unwrap_or_else(|| panic!("the child printed its {key}:\n{stdout}"));
    let cfg = config();

    let database = child("database");
    let mut admin = PgConnection::connect(&cfg.database_url).await.unwrap();
    let database_left: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
        .bind(database)
        .fetch_one(&mut admin)
        .await
        .unwrap();
    assert!(!database_left, "{database} was dropped");

    let nats_prefix = child("nats_prefix");
    let js = async_nats::jetstream::new(async_nats::connect(&cfg.nats_url).await.unwrap());
    let streams: Vec<String> = js.stream_names().try_collect().await.unwrap();
    let streams_left: Vec<&String> =
        streams.iter().filter(|name| name.starts_with(&format!("{nats_prefix}_"))).collect();
    assert!(streams_left.is_empty(), "the streams of {nats_prefix} were deleted: {streams_left:?}");

    let analytics = child("clickhouse_database");
    let clickhouse = clickhouse::Client::default()
        .with_url(&cfg.clickhouse_url)
        .with_user(&cfg.clickhouse_user)
        .with_password(&cfg.clickhouse_password);
    let analytics_left: u64 = clickhouse
        .query("SELECT count() FROM system.databases WHERE name = ?")
        .bind(analytics)
        .fetch_one()
        .await
        .unwrap();
    assert_eq!(analytics_left, 0, "{analytics} was dropped");

    let s3 = aws_sdk_s3::Client::from_conf(
        aws_sdk_s3::Config::builder()
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .region(aws_sdk_s3::config::Region::new(cfg.s3_region.clone()))
            .endpoint_url(&cfg.s3_endpoint)
            .credentials_provider(aws_sdk_s3::config::Credentials::new(
                &cfg.s3_access_key,
                &cfg.s3_secret_key,
                None,
                None,
                "at-exit",
            ))
            .force_path_style(true)
            .build(),
    );
    for bucket in [child("media_bucket"), child("private_bucket")] {
        assert!(bucket.starts_with("tokuly-test-"), "the child had buckets of its own: {bucket}");
        assert!(s3.head_bucket().bucket(bucket).send().await.is_err(), "{bucket} was deleted");
    }
}

#[tokio::test]
async fn child_starts_an_app_and_panics() {
    if std::env::var(CHILD).is_err() {
        return;
    }
    let app = TestApp::start(&["0001", "0100", "0400"], |_, _, router| router).await;
    let cfg = &app.core.cfg;
    let database = cfg.database_url.rsplit('/').next().unwrap().split('?').next().unwrap();
    println!("child-run database={database}");
    println!("child-run nats_prefix={}", cfg.nats_prefix);
    println!("child-run clickhouse_database={}", cfg.clickhouse_database);
    // Something to empty before the bucket can go.
    let upload = app.core.storage.put(Bucket::Private, "left/behind.txt", Bytes::from_static(b"x"), "text/plain").await;
    upload.expect("upload to the run's bucket");
    println!("child-run media_bucket={}", cfg.s3_media_bucket);
    println!("child-run private_bucket={}", cfg.s3_private_bucket);
    panic!("the child's test fails");
}
