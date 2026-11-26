//! The run's NATS namespace, `T<started>_<id>`: its subjects and its
//! JetStream streams (`T<started>_<id>_EVENTS`, …), whose consumers go with
//! them.
use futures::TryStreamExt as _;
use tokuly_core::config::Config;

use crate::run::{self, Run};

const HEAD: &str = "T";

pub(crate) fn prefix(run: &Run) -> String {
    run.name(HEAD, '_')
}

async fn jetstream(cfg: &Config) -> anyhow::Result<async_nats::jetstream::Context> {
    Ok(async_nats::jetstream::new(async_nats::connect(&cfg.nats_url).await?))
}

async fn delete_streams(js: &async_nats::jetstream::Context, names: &[String]) -> anyhow::Result<()> {
    for name in names {
        js.delete_stream(name).await?;
    }
    Ok(())
}

pub(crate) async fn remove(cfg: &Config, run: &Run) -> anyhow::Result<()> {
    let js = jetstream(cfg).await?;
    let names: Vec<String> = js.stream_names().try_collect().await?;
    let head = format!("{}_", prefix(run));
    let ours: Vec<String> = names.into_iter().filter(|name| name.starts_with(&head)).collect();
    delete_streams(&js, &ours).await
}

/// Deletes the streams of stale runs; returns how many.
pub(crate) async fn sweep(cfg: &Config, now: u64) -> anyhow::Result<usize> {
    let js = jetstream(cfg).await?;
    let names: Vec<String> = js.stream_names().try_collect().await?;
    let mut deleted = 0;
    for name in names.iter().filter(|name| run::is_stale(name, HEAD, '_', now)) {
        // Another binary starting at the same moment may delete it first.
        if js.delete_stream(name).await.is_ok() {
            deleted += 1;
        }
    }
    Ok(deleted)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::time::Duration;

    use tokuly_core::bus::{Bus, STREAM_EVENTS};

    use super::*;
    use crate::{env::base_config, run::STALE_AFTER};

    pub(crate) async fn streams_of(cfg: &Config, prefix: &str) -> Vec<String> {
        let js = jetstream(cfg).await.expect("NATS");
        let names: Vec<String> = js.stream_names().try_collect().await.expect("list streams");
        names.into_iter().filter(|name| name.starts_with(&format!("{prefix}_"))).collect()
    }

    /// The streams a real Core makes for `prefix`, with a consumer on one.
    async fn create(cfg: &Config, prefix: &str) {
        let bus = Bus::connect(&cfg.nats_url, prefix).await.expect("bus");
        bus.ensure_streams().await.expect("streams");
        bus.durable_consumer(STREAM_EVENTS, "testkit-cleanup", vec!["evt.>".to_owned()], Duration::from_secs(30), 1)
            .await
            .expect("consumer");
    }

    #[tokio::test]
    async fn a_run_removes_its_streams_and_consumers() {
        let cfg = base_config();
        let run = Run::new();
        create(&cfg, &prefix(&run)).await;
        assert!(!streams_of(&cfg, &prefix(&run)).await.is_empty());
        remove(&cfg, &run).await.unwrap();
        assert_eq!(streams_of(&cfg, &prefix(&run)).await, Vec::<String>::new());
    }

    #[tokio::test]
    async fn the_sweep_takes_only_stale_test_streams() {
        let cfg = base_config();
        let now = run::unix_now();
        let stale = prefix(&Run::started_at(now - STALE_AFTER.as_secs() - 60));
        let live = prefix(&Run::started_at(now - 60));
        // What `dev.sh api-here` gives an agent's own API.
        let foreign = format!("sweep{}", Run::new().name("", '_'));
        for prefix in [&stale, &live, &foreign] {
            create(&cfg, prefix).await;
        }
        sweep(&cfg, now).await.unwrap();
        assert!(streams_of(&cfg, &stale).await.is_empty());
        assert!(!streams_of(&cfg, &live).await.is_empty());
        assert!(!streams_of(&cfg, &foreign).await.is_empty());
        let js = jetstream(&cfg).await.unwrap();
        for prefix in [&live, &foreign] {
            delete_streams(&js, &streams_of(&cfg, prefix).await).await.unwrap();
        }
    }
}
