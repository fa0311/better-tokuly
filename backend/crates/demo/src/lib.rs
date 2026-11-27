//! The demo simulator (`tokuly demo`): a small town of bots, so that a
//! first-time visitor sees what the app is like with friends in it.
//!
//! The bots walk and cycle around a region of the map, chat with each
//! other, post notes, stories and shorts and react to them. Whoever asks
//! one of them to be friends is accepted, greeted and introduced to a few
//! more.
//!
//! How it is built:
//!
//! - **An ordinary client.** The simulator talks to the public API over
//!   HTTP ([`api`]) and writes nothing to a database. Its accounts are
//!   signed up like anyone's, the bots are made on the developer platform
//!   and act with their own API tokens, and every rule, audience and rate
//!   limit of the server applies to them. The one thing the server grants
//!   on purpose is in `tokuly-dev`: bots of the accounts named in
//!   `TOKULY_DEV_DEMO_OPERATORS` may hold write scopes ([`setup`]).
//! - **Marked as bots.** The accounts carry the bot flag (profiles show it
//!   with their owner), their bios start with 「デモ用のボットです🤖」, and
//!   the positions they report are flagged as simulated.
//! - **Decided by a seed.** Who the bots are, where they go and what they
//!   say comes out of [`script::Script`], which talks to nobody and tells
//!   the same story for the same seed and episode (the episode is the second
//!   of the start unless one is named, so a restart goes on with new
//!   lines). [`runner`] plays it against the server; only the answers to
//!   visitors depend on what people do.
//! - **Off unless asked for.** Without operator addresses and a password
//!   the command explains what to set and exits.
pub mod api;
pub mod cast;
pub mod config;
pub mod keep;
pub mod lines;
pub mod media;
pub mod posts;
pub mod rng;
pub mod runner;
pub mod script;
pub mod setup;
pub mod talk;
pub mod town;
pub mod walk;

use std::{future::Future, sync::Arc, time::Duration};

pub use config::DemoConfig;

use crate::{api::Api, runner::Tally, script::Script};

/// Sets the demo up and runs it until `stop` completes (or for
/// `run_seconds`). Returns what it did.
pub async fn run(config: DemoConfig, stop: impl Future<Output = ()>) -> anyhow::Result<Arc<Tally>> {
    config.validate()?;
    let api = Api::new(&config.api_url, config.storage_url.as_deref())?;
    let episode = config.episode();
    let script = Script::new(config.seed, episode, config.bots, config.region());
    tokio::pin!(stop);
    let bots = tokio::select! {
        bots = prepare(&api, &config, &script) => bots?,
        _ = &mut stop => anyhow::bail!("stopped while setting up"),
    };
    tracing::info!(bots = bots.len(), seed = config.seed, episode, center = ?config.center.0, radius_m = config.radius_m, "demo: the bots are out");
    let limit = async {
        match config.run_seconds {
            0 => std::future::pending().await,
            seconds => tokio::time::sleep(Duration::from_secs(seconds)).await,
        }
    };
    let stop = async {
        tokio::select! {
            _ = stop => {}
            _ = limit => {}
        }
    };
    let tally = runner::run(api, script, bots, config.pace, config.companions, config.seed, stop).await;
    tracing::info!("demo: stopped after {tally}");
    Ok(tally)
}

/// Sets the cast up, waiting for a server that is still starting and
/// waiting out a refusal for too many requests: exiting would only restart
/// the container into the same refusal.
async fn prepare(api: &Api, config: &DemoConfig, script: &Script) -> anyhow::Result<Vec<setup::Bot>> {
    let mut waited = Duration::ZERO;
    let mut refusals = 0;
    loop {
        match setup::prepare(api, config, script).await {
            Err(error) if waited < Duration::from_secs(180) && refused(&error, "UNREACHABLE") => {
                tracing::info!(api = %config.api_url, "demo: waiting for the API");
                tokio::time::sleep(Duration::from_secs(5)).await;
                waited += Duration::from_secs(5);
            }
            Err(error) if refused(&error, "RATE_LIMITED") => {
                let pause = backoff(refusals);
                refusals += 1;
                tracing::warn!(
                    error = format!("{error:#}"),
                    retry_in_s = pause.as_secs(),
                    "demo: the API asks to slow down"
                );
                tokio::time::sleep(pause).await;
            }
            done => return done,
        }
    }
}

/// How long to wait after the `refusals`-th refusal in a row: a minute,
/// doubling up to a quarter of an hour.
fn backoff(refusals: u32) -> Duration {
    Duration::from_secs(60 << refusals.min(4)).min(Duration::from_secs(15 * 60))
}

fn refused(error: &anyhow::Error, reason: &str) -> bool {
    error.chain().any(|cause| cause.downcast_ref::<api::ApiError>().is_some_and(|refused| refused.is(reason)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_are_waited_out_longer_and_longer() {
        let minutes: Vec<u64> = (0..7).map(|n| backoff(n).as_secs() / 60).collect();
        assert_eq!(minutes, [1, 2, 4, 8, 15, 15, 15]);
    }
}
