//! Where events and jobs go when their handler gave up on them.
//!
//! A consumer retries a failing event (and a worker a failing job) with a
//! growing delay; after the last attempt the message used to be dropped
//! with a log line. An outage that outlasts the retries (the database, the
//! search engine, a bug in one handler) would then lose, for good, whatever
//! was in flight: an account deletion another domain never heard of, a
//! notification, a reward. Now the message is *buried* first: stored on the
//! `DEAD` stream with the consumer it failed in and the last error, kept for
//! [`RETENTION`], and an operator sends it round again once the cause is
//! fixed:
//!
//! ```text
//! tokuly dead-letters list
//! tokuly dead-letters redrive            # everything
//! tokuly dead-letters redrive notify     # what failed in consumers named notify…
//! ```
//!
//! A redriven event goes back on its topic addressed to the consumer it
//! failed in ([`REDRIVE_FOR`]): the other consumers of the topic handled it
//! long ago and pass it by. They must, because a letter can be a month old
//! and the records consumers keep of what they handled are not (notify and
//! feed remember event IDs for a week): sent to everyone, a redrive would
//! notify people again of what happened weeks ago. A redriven job goes back
//! on its queue.
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    bus::{Bus, STREAM_DEAD},
    error::{AppError, AppResult},
};

/// The header naming the one consumer a redriven event is for.
pub const REDRIVE_FOR: &str = "Tokuly-Redrive-For";

/// Whether `message` is an event sent round again for a consumer other than
/// `consumer`, which then lets it pass.
pub(crate) fn redriven_for_another(message: &async_nats::Message, consumer: &str) -> bool {
    message.headers.as_ref().and_then(|h| h.get(REDRIVE_FOR)).is_some_and(|target| target.as_str() != consumer)
}

/// How long a buried message waits for an operator.
pub const RETENTION: Duration = Duration::from_secs(30 * 24 * 3600);

/// The most text kept of the error a message died of.
const MAX_ERROR_CHARS: usize = 2_000;

/// A message its handler gave up on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeadLetter {
    /// The durable consumer (`notify-events`) or job queue
    /// (`job-media-transcode`) it failed in.
    pub consumer: String,
    /// The subject it was published on, without the deployment's namespace
    /// (`evt.identity.user.deleted`, `job.media.transcode`): where a redrive
    /// sends it.
    pub subject: String,
    /// The last error, or why it could not be read.
    pub error: String,
    /// How often it was delivered before the handler gave up.
    pub deliveries: i64,
    pub died_at: DateTime<Utc>,
    /// The message as it was published (every event and job is JSON).
    pub payload: serde_json::Value,
    /// The message as text, when it was not JSON (`payload` is then null).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

/// A buried message and its place on the `DEAD` stream.
#[derive(Debug, Clone)]
pub struct Buried {
    pub sequence: u64,
    pub letter: DeadLetter,
}

/// What a redrive did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Redriven {
    /// Messages sent round again and removed from the `DEAD` stream.
    pub sent: usize,
    /// Messages that could not be sent and stay buried.
    pub failed: usize,
}

/// The subject a message that died in `consumer` is buried under.
fn grave(consumer: &str) -> String {
    let consumer: String = consumer.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    format!("dead.{consumer}")
}

/// Stores a message its handler gave up on. `subject` is the subject as
/// received (with the deployment's namespace).
pub async fn bury(bus: &Bus, consumer: &str, subject: &str, payload: &[u8], error: &str, deliveries: i64) -> AppResult<()> {
    let json: Option<serde_json::Value> = serde_json::from_slice(payload).ok();
    let letter = DeadLetter {
        consumer: consumer.to_owned(),
        subject: bus.plain_subject(subject).to_owned(),
        error: error.chars().take(MAX_ERROR_CHARS).collect(),
        deliveries,
        died_at: Utc::now(),
        raw: json.is_none().then(|| String::from_utf8_lossy(payload).into_owned()),
        payload: json.unwrap_or_default(),
    };
    let body = serde_json::to_vec(&letter).map_err(|e| AppError::internal(anyhow::anyhow!("dead letter encode: {e}")))?;
    bus.publish_durable(grave(consumer), body, None).await
}

/// [`bury`] for the consumers: a message that cannot be buried is lost, as
/// every such message was before, and the log says so.
pub(crate) async fn bury_or_log(bus: &Bus, consumer: &str, subject: &str, payload: &[u8], error: &str, deliveries: i64) {
    match bury(bus, consumer, subject, payload, error, deliveries).await {
        Ok(()) => tracing::error!(consumer, subject, deliveries, error, "gave up on a message; buried (see `tokuly dead-letters list`)"),
        Err(e) => tracing::error!(consumer, subject, deliveries, error, bury_error = %e, "gave up on a message and could not bury it; lost"),
    }
}

/// The buried messages, oldest first, at most `limit`; only those that
/// failed in a consumer whose name starts with `consumer_prefix` when one
/// is given.
pub async fn list(bus: &Bus, consumer_prefix: Option<&str>, limit: usize) -> AppResult<Vec<Buried>> {
    let unavailable = |e: String| AppError::unavailable("BUS_UNAVAILABLE", format!("dead letters cannot be read: {e}"));
    let mut stream = bus.js.get_stream(bus.stream_name(STREAM_DEAD)).await.map_err(|e| unavailable(e.to_string()))?;
    let state = stream.info().await.map_err(|e| unavailable(e.to_string()))?.state.clone();
    let mut buried = Vec::new();
    if state.messages == 0 {
        return Ok(buried);
    }
    for sequence in state.first_sequence..=state.last_sequence {
        if buried.len() >= limit {
            break;
        }
        // Sequences of messages already redriven or expired are gone.
        let Ok(message) = stream.get_raw_message(sequence).await else { continue };
        let Ok(letter) = serde_json::from_slice::<DeadLetter>(&message.payload) else { continue };
        if consumer_prefix.is_none_or(|prefix| letter.consumer.starts_with(prefix)) {
            buried.push(Buried { sequence, letter });
        }
    }
    Ok(buried)
}

/// Sends buried messages round again, each to the consumer it failed in
/// (see the module documentation), and removes the ones that were sent. Run it once the cause of the failures
/// is fixed: a message that fails again is buried again.
pub async fn redrive(bus: &Bus, consumer_prefix: Option<&str>, limit: usize) -> AppResult<Redriven> {
    let stream = bus
        .js
        .get_stream(bus.stream_name(STREAM_DEAD))
        .await
        .map_err(|e| AppError::unavailable("BUS_UNAVAILABLE", format!("dead letters cannot be read: {e}")))?;
    let mut done = Redriven::default();
    for Buried { sequence, letter } in list(bus, consumer_prefix, limit).await? {
        let payload = match &letter.raw {
            Some(text) => text.clone().into_bytes(),
            None => serde_json::to_vec(&letter.payload).map_err(|e| AppError::internal(anyhow::anyhow!("dead letter payload: {e}")))?,
        };
        // No deduplication ID: the stream may still remember the original's.
        let mut headers = async_nats::HeaderMap::new();
        headers.insert(REDRIVE_FOR, letter.consumer.as_str());
        match bus.publish_durable_with_headers(letter.subject.clone(), headers, payload).await {
            Ok(()) => {
                if let Err(e) = stream.delete_message(sequence).await {
                    tracing::warn!(sequence, error = %e, "redriven message could not be removed from the dead letters");
                }
                done.sent += 1;
            }
            Err(e) => {
                tracing::warn!(sequence, subject = %letter.subject, error = %e, "dead letter could not be sent again");
                done.failed += 1;
            }
        }
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graves_are_one_subject_token_per_consumer() {
        assert_eq!(grave("notify-events"), "dead.notify-events");
        assert_eq!(grave("job-media-transcode"), "dead.job-media-transcode");
        assert_eq!(grave("odd.name with*wild>cards"), "dead.odd_name_with_wild_cards");
    }
}
