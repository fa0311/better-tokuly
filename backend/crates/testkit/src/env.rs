//! The shared infrastructure as one test binary sees it: what the run
//! creates when its first test starts, and removes when the process exits.
//!
//! - start-up: sweep what crashed runs left behind ([`run::STALE_AFTER`]),
//!   then create this run's resources;
//! - exit: remove them again ([`run::at_exit`]), whether the tests passed or
//!   panicked. Each removal is bounded by [`REMOVE_TIMEOUT`], so a service
//!   that went away cannot hang the end of `cargo test`.
use std::{sync::OnceLock, time::Duration};

use clap::Parser;
use tokuly_core::config::Config;

use crate::{
    buckets, clickhouse, nats, postgres,
    run::{self, Run},
    search_indexes,
};

const REMOVE_TIMEOUT: Duration = Duration::from_secs(10);

/// Everything shared by the tests of one test binary.
pub(crate) struct TestEnv {
    /// The configuration pointing at this run's resources.
    pub cfg: Config,
    /// ClickHouse was reachable and has the run's database.
    clickhouse: bool,
    /// The object store was reachable and has the run's buckets.
    buckets: bool,
}

static ENV: OnceLock<TestEnv> = OnceLock::new();

#[derive(Parser)]
struct EnvConfig {
    #[command(flatten)]
    core: Config,
}

pub(crate) fn base_config() -> Config {
    // Reads TOKULY_* from the environment; the secret gets a test default.
    let mut args = vec!["tokuly-test".to_owned()];
    if std::env::var("TOKULY_SECRET").is_err() {
        args.push("--secret".to_owned());
        args.push("test-secret-test-secret-test-secret-test".to_owned());
    }
    EnvConfig::parse_from(args).core
}

pub(crate) fn run() -> &'static Run {
    static RUN: OnceLock<Run> = OnceLock::new();
    RUN.get_or_init(Run::new)
}

/// The environment of this test binary, set up by the first caller with
/// its `migrations`.
pub(crate) fn get(migrations: &[&str]) -> &'static TestEnv {
    let migrations: Vec<String> = migrations.iter().map(|m| m.to_string()).collect();
    ENV.get_or_init(|| {
        // Each #[tokio::test] has its own runtime; do the one-time setup on a dedicated one.
        std::thread::spawn(move || block_on(set_up(migrations))).join().expect("testkit: environment setup panicked")
    })
}

fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread().enable_all().build().expect("testkit: runtime").block_on(future)
}

async fn set_up(migrations: Vec<String>) -> TestEnv {
    let base = base_config();
    sweep(&base).await;
    // Registered before anything exists, so a setup that panics half-way is
    // cleaned up too.
    let exit_cfg = base.clone();
    run::at_exit(move || {
        let cfg = exit_cfg.clone();
        let _ = std::thread::spawn(move || block_on(remove(&cfg))).join();
    });

    let run = run();
    let mut cfg = base.clone();
    cfg.database_url = postgres::create(&base, run, &migrations).await;
    cfg.nats_prefix = nats::prefix(run);
    let clickhouse = match clickhouse::create(&base, run).await {
        Some(database) => {
            cfg.clickhouse_database = database;
            true
        }
        None => false,
    };
    let buckets = match buckets::create(&cfg, run).await {
        Some(with_buckets) => {
            cfg = with_buckets;
            true
        }
        None => false,
    };
    TestEnv { cfg, clickhouse, buckets }
}

/// Removes leftovers of crashed runs. Best effort: a service that is down
/// shows up when the run creates its own resources there.
async fn sweep(base: &Config) {
    let now = run::unix_now();
    let _ = tokio::join!(
        postgres::sweep(base, now),
        nats::sweep(base, now),
        clickhouse::sweep(base, now),
        buckets::sweep(base, now),
        search_indexes::sweep(base, now),
    );
}

/// Removes what this run created.
async fn remove(base: &Config) {
    let run = run();
    // When setup panicked there is no environment: try everything.
    let env = ENV.get();
    let (database, streams, analytics, objects, indexes) = tokio::join!(
        bounded(postgres::remove(base, run)),
        bounded(nats::remove(base, run)),
        bounded_if(env.is_none_or(|env| env.clickhouse), clickhouse::remove(base, run)),
        bounded_if(env.is_none_or(|env| env.buckets), buckets::remove(base, run)),
        bounded(search_indexes::remove(base, run)),
    );
    report("PostgreSQL database", database);
    report("JetStream streams", streams);
    report("ClickHouse database", analytics);
    report("buckets", objects);
    report("Meilisearch indexes", indexes);
}

async fn bounded_if(created: bool, removal: impl Future<Output = anyhow::Result<()>>) -> anyhow::Result<()> {
    if created { bounded(removal).await } else { Ok(()) }
}

async fn bounded(removal: impl Future<Output = anyhow::Result<()>>) -> anyhow::Result<()> {
    tokio::time::timeout(REMOVE_TIMEOUT, removal).await.unwrap_or_else(|_| Err(anyhow::anyhow!("timed out")))
}

fn report(what: &str, result: anyhow::Result<()>) {
    if let Err(e) = result {
        eprintln!(
            "testkit: could not remove the test run's {what} ({e:#}); a run after {} h removes it",
            run::STALE_AFTER.as_secs() / 3600
        );
    }
}
