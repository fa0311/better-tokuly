//! The run's Meilisearch indexes: every index whose name starts with
//! [`crate::search_index_prefix`] (`tokuly_test_<started>_<id>`). Only the
//! search tests create any, so nothing is asked of Meilisearch at exit
//! unless the prefix was handed out.
use std::sync::atomic::{AtomicBool, Ordering};

use meilisearch_sdk::{client::Client, indexes::IndexesQuery};
use tokuly_core::config::Config;

use crate::run::{self, Run};

const HEAD: &str = "tokuly_test_";
const PAGE: usize = 1000;

static HANDED_OUT: AtomicBool = AtomicBool::new(false);

/// The prefix, marked as in use.
pub(crate) fn prefix(run: &Run) -> String {
    HANDED_OUT.store(true, Ordering::Relaxed);
    name(run)
}

fn name(run: &Run) -> String {
    run.name(HEAD, '_')
}

fn client(cfg: &Config) -> anyhow::Result<Client> {
    Ok(Client::new(&cfg.meili_url, Some(&cfg.meili_key))?)
}

async fn index_names(client: &Client) -> anyhow::Result<Vec<String>> {
    let mut names = Vec::new();
    loop {
        let mut query = IndexesQuery::new(client);
        query.with_limit(PAGE).with_offset(names.len());
        let page = query.execute().await?;
        let done = page.results.len() < PAGE;
        names.extend(page.results.into_iter().map(|index| index.uid));
        if done {
            return Ok(names);
        }
    }
}

/// Deletes the indexes under `prefix` (deletion finishes inside Meilisearch).
async fn delete_under(client: &Client, prefix: &str) -> anyhow::Result<()> {
    for uid in index_names(client).await?.iter().filter(|uid| uid.starts_with(prefix)) {
        client.delete_index(uid).await?;
    }
    Ok(())
}

pub(crate) async fn remove(cfg: &Config, run: &Run) -> anyhow::Result<()> {
    if !HANDED_OUT.load(Ordering::Relaxed) {
        return Ok(());
    }
    delete_under(&client(cfg)?, &name(run)).await
}

/// Deletes the indexes of stale runs; returns how many.
pub(crate) async fn sweep(cfg: &Config, now: u64) -> anyhow::Result<usize> {
    let client = client(cfg)?;
    let mut deleted = 0;
    for uid in index_names(&client).await?.iter().filter(|uid| run::is_stale(uid, HEAD, '_', now)) {
        if client.delete_index(uid).await.is_ok() {
            deleted += 1;
        }
    }
    Ok(deleted)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{env::base_config, run::STALE_AFTER};

    async fn create_index(client: &Client, uid: &str) {
        let task = client.create_index(uid, Some("id")).await.expect("create index");
        task.wait_for_completion(client, None, Some(Duration::from_secs(10))).await.expect("index created");
    }

    /// Whether `uid` exists once Meilisearch has caught up with deletions.
    async fn exists(client: &Client, uid: &str) -> bool {
        for _ in 0..50 {
            if !index_names(client).await.expect("list indexes").iter().any(|name| name == uid) {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        true
    }

    #[tokio::test]
    async fn a_run_removes_its_indexes() {
        let cfg = base_config();
        let client = client(&cfg).unwrap();
        let run = Run::new();
        let uid = format!("{}_users", prefix(&run));
        create_index(&client, &uid).await;
        remove(&cfg, &run).await.unwrap();
        assert!(!exists(&client, &uid).await, "{uid} was deleted");
    }

    #[tokio::test]
    async fn the_sweep_takes_only_stale_test_indexes() {
        let cfg = base_config();
        let client = client(&cfg).unwrap();
        let now = run::unix_now();
        let stale = format!("{}_users", name(&Run::started_at(now - STALE_AFTER.as_secs() - 60)));
        let live = format!("{}_users", name(&Run::started_at(now - 60)));
        // What `dev.sh api-here` gives an agent's own API.
        let foreign = format!("sweep{}__users", Run::new().name("", '_'));
        for uid in [&stale, &live, &foreign] {
            create_index(&client, uid).await;
        }
        sweep(&cfg, now).await.unwrap();
        assert!(!exists(&client, &stale).await, "{stale} was swept");
        for uid in [&live, &foreign] {
            assert!(index_names(&client).await.unwrap().contains(uid), "{uid} was kept");
            client.delete_index(uid).await.unwrap();
        }
    }
}
