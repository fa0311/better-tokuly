//! Per-user ID lists cached in Valkey: a user's friends, and everyone hidden
//! from a user by a block.
//!
//! Other domains read these lists on their hot paths (location fan-out,
//! feed and search filtering), so they are served from Valkey and only fall
//! back to PostgreSQL on a miss. The database stays the source of truth:
//!
//! - Writers call [`IdList::invalidate`] after committing. It drops the cached
//!   list *and* leaves a short-lived "dirty" marker.
//! - Readers that loaded the list from the database store it only if no
//!   marker exists (checked atomically in a script). A reader that raced a
//!   writer and holds a pre-commit snapshot therefore cannot put it back.
//! - Entries expire on their own, which bounds the damage if an invalidation
//!   is ever lost (for instance when Valkey is briefly unreachable).
//!
//! A Valkey failure never fails the caller: the list is simply read from the
//! database.
use std::future::Future;

use redis::AsyncCommands as _;
use tokuly_core::prelude::*;

/// How long a cached list lives without being invalidated.
const TTL_SECONDS: u64 = 600;
/// How long after a change readers must not repopulate the cache. Only needs
/// to outlast one database read.
const DIRTY_SECONDS: u64 = 5;

/// Stores `ARGV[1]` under `KEYS[1]` unless the dirty marker `KEYS[2]` exists.
const STORE_UNLESS_DIRTY: &str = r"
if redis.call('EXISTS', KEYS[2]) == 0 then
    redis.call('SET', KEYS[1], ARGV[1], 'EX', ARGV[2])
    return 1
end
return 0";

/// One kind of cached per-user ID list.
pub(crate) struct IdList {
    prefix: &'static str,
}

/// IDs of a user's friends.
pub(crate) const FRIENDS: IdList = IdList { prefix: "social:friends" };
/// IDs of everyone a user blocked or was blocked by.
pub(crate) const HIDDEN: IdList = IdList { prefix: "social:hidden" };

impl IdList {
    fn key(&self, user_id: Uuid) -> String {
        format!("{}:{}", self.prefix, user_id.simple())
    }

    fn dirty_key(&self, user_id: Uuid) -> String {
        format!("{}:dirty:{}", self.prefix, user_id.simple())
    }

    /// The cached list, or the result of `load` (which is then cached).
    pub(crate) async fn get_or_load<F, Fut>(&self, core: &Core, user_id: Uuid, load: F) -> AppResult<Vec<Uuid>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = AppResult<Vec<Uuid>>>,
    {
        match self.read(core, user_id).await {
            Ok(Some(ids)) => return Ok(ids),
            Ok(None) => {}
            Err(e) => tracing::warn!(list = self.prefix, error = %e, "id list cache read failed; using the database"),
        }
        let ids = load().await?;
        if let Err(e) = self.store(core, user_id, &ids).await {
            tracing::warn!(list = self.prefix, error = %e, "id list cache write failed");
        }
        Ok(ids)
    }

    async fn read(&self, core: &Core, user_id: Uuid) -> AppResult<Option<Vec<Uuid>>> {
        let raw: Option<Vec<u8>> = core.cache.conn().get(self.key(user_id)).await?;
        Ok(raw.and_then(|bytes| decode(&bytes)))
    }

    async fn store(&self, core: &Core, user_id: Uuid, ids: &[Uuid]) -> AppResult<()> {
        redis::Script::new(STORE_UNLESS_DIRTY)
            .key(self.key(user_id))
            .key(self.dirty_key(user_id))
            .arg(encode(ids))
            .arg(TTL_SECONDS)
            .invoke_async::<i64>(&mut core.cache.conn())
            .await?;
        Ok(())
    }

    /// Drops the cached lists of `user_ids`. Call after the transaction that
    /// changed them has committed. Never fails the caller: the change is
    /// already durable, and entries expire by themselves.
    pub(crate) async fn invalidate(&self, core: &Core, user_ids: &[Uuid]) {
        if user_ids.is_empty() {
            return;
        }
        let mut pipe = redis::pipe();
        for user_id in user_ids {
            pipe.cmd("SET").arg(self.dirty_key(*user_id)).arg(1).arg("EX").arg(DIRTY_SECONDS).ignore();
            pipe.cmd("DEL").arg(self.key(*user_id)).ignore();
        }
        if let Err(e) = pipe.query_async::<()>(&mut core.cache.conn()).await {
            tracing::warn!(list = self.prefix, error = %e, "id list cache invalidation failed; entries expire by themselves");
        }
    }
}

/// IDs as their 16 raw bytes back to back: compact and trivially parsed.
fn encode(ids: &[Uuid]) -> Vec<u8> {
    ids.iter().flat_map(|id| *id.as_bytes()).collect()
}

/// `None` for anything that is not a whole number of IDs (treated as a miss).
fn decode(bytes: &[u8]) -> Option<Vec<Uuid>> {
    let (ids, leftover) = bytes.as_chunks::<16>();
    leftover.is_empty().then(|| ids.iter().map(|id| Uuid::from_bytes(*id)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_lists_roundtrip_through_bytes() {
        let ids = vec![Uuid::now_v7(), Uuid::now_v7(), Uuid::nil()];
        assert_eq!(decode(&encode(&ids)), Some(ids));
        assert_eq!(decode(&[]), Some(Vec::new()), "an empty list is a hit, not a miss");
        assert_eq!(decode(&[1, 2, 3]), None);
    }
}
