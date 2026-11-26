//! Valkey: ephemeral state, caches, rate limits and short-lived tokens.
//!
//! Key naming: `<domain>:<thing>:<id>`, e.g. `geo:loc:<user_id>`. Domains use
//! [`Cache::conn`] for arbitrary commands; the helpers here cover the common
//! cases.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use redis::{AsyncCommands as _, aio::ConnectionManager};
use serde::{Serialize, de::DeserializeOwned};

use crate::error::{AppError, AppResult};

/// Counts a hit in the window of `KEYS[1]` (`ARGV[1]` seconds) and returns
/// the count. One step on the server: a counter can never be left without
/// an expiry (a process stopped between an `INCR` and its `EXPIRE` would
/// leave one that only grows, and whoever it counts locked out for good).
/// A counter found without an expiry is given one, which heals those left
/// by releases that counted in two steps.
const COUNT_IN_WINDOW: &str = r"
local count = redis.call('INCR', KEYS[1])
if count == 1 or redis.call('TTL', KEYS[1]) < 0 then
    redis.call('EXPIRE', KEYS[1], ARGV[1])
end
return count
";

#[derive(Clone)]
pub struct Cache {
    conn: ConnectionManager,
    /// `false` turns [`Cache::rate_limit`] into a no-op (`TOKULY_RATE_LIMITS`).
    /// Shared by every clone, so the console can switch it while running.
    rate_limits: Arc<AtomicBool>,
}

impl Cache {
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let client =
            redis::Client::open(url).map_err(|e| anyhow::anyhow!("invalid Valkey URL: {e}"))?;
        let conn = ConnectionManager::new(client)
            .await
            .map_err(|e| anyhow::anyhow!("cannot connect to Valkey at {url}: {e}"))?;
        Ok(Self { conn, rate_limits: Arc::new(AtomicBool::new(true)) })
    }

    /// Whether [`Cache::rate_limit`] enforces its limits (it does by default).
    pub fn with_rate_limits(self, enforce: bool) -> Self {
        self.set_rate_limits(enforce);
        self
    }

    /// Switches the limits on or off for every clone of this cache.
    pub fn set_rate_limits(&self, enforce: bool) {
        self.rate_limits.store(enforce, Ordering::Relaxed);
    }

    /// Whether [`Cache::rate_limit`] enforces its limits right now.
    pub fn enforces_rate_limits(&self) -> bool {
        self.rate_limits.load(Ordering::Relaxed)
    }

    /// A cheap clone of the multiplexed connection for raw commands.
    pub fn conn(&self) -> ConnectionManager {
        self.conn.clone()
    }

    pub async fn get_json<T: DeserializeOwned>(&self, key: &str) -> AppResult<Option<T>> {
        let raw: Option<Vec<u8>> = self.conn().get(key).await?;
        match raw {
            None => Ok(None),
            Some(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
        }
    }

    /// Stores a JSON value; `ttl` of `None` keeps it until deleted.
    pub async fn set_json<T: Serialize + ?Sized>(
        &self,
        key: &str,
        value: &T,
        ttl: Option<Duration>,
    ) -> AppResult<()> {
        let bytes = serde_json::to_vec(value)
            .map_err(|e| AppError::internal(anyhow::anyhow!("cache encode: {e}")))?;
        let mut conn = self.conn();
        match ttl {
            Some(ttl) => {
                conn.set_ex::<_, _, ()>(key, bytes, ttl.as_secs().max(1))
                    .await?
            }
            None => conn.set::<_, _, ()>(key, bytes).await?,
        }
        Ok(())
    }

    pub async fn get_string(&self, key: &str) -> AppResult<Option<String>> {
        Ok(self.conn().get(key).await?)
    }

    pub async fn set_string(&self, key: &str, value: &str, ttl: Option<Duration>) -> AppResult<()> {
        let mut conn = self.conn();
        match ttl {
            Some(ttl) => {
                conn.set_ex::<_, _, ()>(key, value, ttl.as_secs().max(1))
                    .await?
            }
            None => conn.set::<_, _, ()>(key, value).await?,
        }
        Ok(())
    }

    /// Reads and deletes a value atomically (one-time tokens, challenges).
    pub async fn take_json<T: DeserializeOwned>(&self, key: &str) -> AppResult<Option<T>> {
        let raw: Option<Vec<u8>> = redis::cmd("GETDEL")
            .arg(key)
            .query_async(&mut self.conn())
            .await?;
        match raw {
            None => Ok(None),
            Some(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
        }
    }

    pub async fn del(&self, key: &str) -> AppResult<()> {
        self.conn().del::<_, ()>(key).await?;
        Ok(())
    }

    pub async fn exists(&self, key: &str) -> AppResult<bool> {
        Ok(self.conn().exists(key).await?)
    }

    /// Sets `key` only if it is absent; returns whether this call set it.
    /// Useful as a short lock or an idempotency marker.
    pub async fn set_nx(&self, key: &str, value: &str, ttl: Duration) -> AppResult<bool> {
        let result: Option<String> = redis::cmd("SET")
            .arg(key)
            .arg(value)
            .arg("NX")
            .arg("EX")
            .arg(ttl.as_secs().max(1))
            .query_async(&mut self.conn())
            .await?;
        Ok(result.is_some())
    }

    /// Fixed-window rate limit: allows `max` hits per `window` for `key`.
    /// Returns a `resource_exhausted` error when exceeded. Always allows when
    /// limits are switched off for this deployment (`TOKULY_RATE_LIMITS=false`).
    pub async fn rate_limit(&self, key: &str, max: u32, window: Duration) -> AppResult<()> {
        if !self.enforces_rate_limits() {
            return Ok(());
        }
        let count: i64 = redis::Script::new(COUNT_IN_WINDOW)
            .key(format!("rl:{key}"))
            .arg(window.as_secs().max(1))
            .invoke_async(&mut self.conn())
            .await?;
        if count > max as i64 {
            return Err(AppError::rate_limited(
                "RATE_LIMITED",
                "too many requests, slow down",
            ));
        }
        Ok(())
    }

    pub async fn ping(&self) -> AppResult<()> {
        redis::cmd("PING")
            .query_async::<String>(&mut self.conn())
            .await?;
        Ok(())
    }
}
