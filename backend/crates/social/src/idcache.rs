//! Per-user lists cached in Valkey: a user's friends (as IDs and with the
//! details other domains ask about), everyone hidden from a user by a block,
//! followers and followees, and circles (their members, and the circles a
//! user is in).
//!
//! Other domains read these lists on their hot paths (location fan-out,
//! feed and search filtering), so they are served from Valkey and only fall
//! back to PostgreSQL on a miss. The database stays the source of truth:
//!
//! - Every list has a generation counter. Writers call
//!   [`IdList::invalidate`] after committing: it bumps the generation and
//!   drops the cached list.
//! - Readers note the generation before they load the list from the
//!   database, and store what they loaded only if the generation is still
//!   the same (checked atomically in a script). A reader that raced a writer
//!   and holds a pre-commit snapshot therefore cannot put it back, however
//!   slow it is.
//! - Entries expire on their own, which bounds the damage if an invalidation
//!   is ever lost (for instance when Valkey is briefly unreachable).
//!
//! A Valkey failure never fails the caller: the list is simply read from the
//! database.
use std::future::Future;

use tokuly_core::prelude::*;

/// How long a cached list lives without being invalidated.
const TTL_SECONDS: u64 = 600;
/// How long a generation counter outlives its last change. Far longer than
/// any database read: a reader that started before the change finds the
/// counter changed (or gone) and does not store.
const GENERATION_SECONDS: u64 = 3600;

/// Stores `ARGV[1]` under `KEYS[1]` (for `ARGV[2]` seconds) if the generation
/// `KEYS[2]` is still `ARGV[3]` (empty: no generation yet).
const STORE_IF_UNCHANGED: &str = r"
if (redis.call('GET', KEYS[2]) or '') == ARGV[3] then
    redis.call('SET', KEYS[1], ARGV[1], 'EX', ARGV[2])
    return 1
end
return 0";

/// A fixed-size entry of a cached list.
pub(crate) trait Record: Sized {
    const SIZE: usize;
    fn write(&self, out: &mut Vec<u8>);
    /// Reads one entry from exactly [`Self::SIZE`] bytes.
    fn read(bytes: &[u8]) -> Option<Self>;
}

impl Record for Uuid {
    const SIZE: usize = 16;

    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.as_bytes());
    }

    fn read(bytes: &[u8]) -> Option<Self> {
        Uuid::from_slice(bytes).ok()
    }
}

/// A pair of IDs, e.g. (circle, owner).
impl Record for (Uuid, Uuid) {
    const SIZE: usize = 32;

    fn write(&self, out: &mut Vec<u8>) {
        self.0.write(out);
        self.1.write(out);
    }

    fn read(bytes: &[u8]) -> Option<Self> {
        Some((Uuid::read(&bytes[..16])?, Uuid::read(&bytes[16..])?))
    }
}

/// One kind of cached per-user (or per-circle) list.
pub(crate) struct IdList {
    prefix: &'static str,
}

/// IDs of a user's friends.
pub(crate) const FRIENDS: IdList = IdList { prefix: "social:friends" };
/// A user's friendships with the details other domains ask about
/// ([`crate::graph::FriendLink`]): since when, best-friend rank, star, mute.
pub(crate) const FRIEND_LINKS: IdList = IdList { prefix: "social:friend-links" };
/// IDs of everyone a user blocked or was blocked by.
pub(crate) const HIDDEN: IdList = IdList { prefix: "social:hidden" };
/// IDs of a user's followers.
pub(crate) const FOLLOWERS: IdList = IdList { prefix: "social:followers" };
/// IDs of the accounts a user follows.
pub(crate) const FOLLOWING: IdList = IdList { prefix: "social:following" };
/// IDs of the members of a circle (keyed by the circle's ID).
pub(crate) const CIRCLE_MEMBERS: IdList = IdList { prefix: "social:circle-members" };
/// The circles a user is a member of, as (circle, owner) pairs.
pub(crate) const MEMBER_OF: IdList = IdList { prefix: "social:member-of" };

/// What a cache read found: the encoded list (on a hit) and the generation
/// it was read at (empty when there is none yet).
struct Cached {
    bytes: Option<Vec<u8>>,
    generation: String,
}

impl IdList {
    fn key(&self, user_id: Uuid) -> String {
        format!("{}:{}", self.prefix, user_id.simple())
    }

    fn generation_key(&self, user_id: Uuid) -> String {
        format!("{}:gen:{}", self.prefix, user_id.simple())
    }

    /// The cached list, or the result of `load` (which is then cached).
    pub(crate) async fn get_or_load<R, F, Fut>(&self, core: &Core, user_id: Uuid, load: F) -> AppResult<Vec<R>>
    where
        R: Record,
        F: FnOnce() -> Fut,
        Fut: Future<Output = AppResult<Vec<R>>>,
    {
        let generation = match self.read(core, user_id).await {
            Ok(Cached { bytes: Some(bytes), generation }) => match decode(&bytes) {
                Some(ids) => return Ok(ids),
                None => Some(generation),
            },
            Ok(Cached { bytes: None, generation }) => Some(generation),
            Err(e) => {
                tracing::warn!(list = self.prefix, error = %e, "id list cache read failed; using the database");
                None
            }
        };
        let ids = load().await?;
        // Without a generation read before the load there is no telling
        // whether the list is still current: do not cache it.
        if let Some(generation) = generation
            && let Err(e) = self.store(core, user_id, &ids, &generation).await
        {
            tracing::warn!(list = self.prefix, error = %e, "id list cache write failed");
        }
        Ok(ids)
    }

    async fn read(&self, core: &Core, user_id: Uuid) -> AppResult<Cached> {
        let (raw, generation): (Option<Vec<u8>>, Option<String>) = redis::cmd("MGET")
            .arg(self.key(user_id))
            .arg(self.generation_key(user_id))
            .query_async(&mut core.cache.conn())
            .await?;
        Ok(Cached { bytes: raw, generation: generation.unwrap_or_default() })
    }

    async fn store<R: Record>(&self, core: &Core, user_id: Uuid, ids: &[R], generation: &str) -> AppResult<()> {
        redis::Script::new(STORE_IF_UNCHANGED)
            .key(self.key(user_id))
            .key(self.generation_key(user_id))
            .arg(encode(ids))
            .arg(TTL_SECONDS)
            .arg(generation)
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
            pipe.cmd("INCR").arg(self.generation_key(*user_id)).ignore();
            pipe.cmd("EXPIRE").arg(self.generation_key(*user_id)).arg(GENERATION_SECONDS).ignore();
            pipe.cmd("DEL").arg(self.key(*user_id)).ignore();
        }
        if let Err(e) = pipe.query_async::<()>(&mut core.cache.conn()).await {
            tracing::warn!(list = self.prefix, error = %e, "id list cache invalidation failed; entries expire by themselves");
        }
    }
}

/// Drops what is cached about `user_ids`' friendships (the ID lists and the
/// details). Call after the transaction that changed them has committed.
pub(crate) async fn friendships_changed(core: &Core, user_ids: &[Uuid]) {
    FRIENDS.invalidate(core, user_ids).await;
    FRIEND_LINKS.invalidate(core, user_ids).await;
}

/// Drops what is cached about follows between followers and followees.
pub(crate) async fn follows_changed(core: &Core, followers: &[Uuid], followees: &[Uuid]) {
    FOLLOWING.invalidate(core, followers).await;
    FOLLOWERS.invalidate(core, followees).await;
}

/// Entries back to back in their fixed-size form: compact and trivially parsed.
fn encode<R: Record>(entries: &[R]) -> Vec<u8> {
    let mut out = Vec::with_capacity(entries.len() * R::SIZE);
    for entry in entries {
        entry.write(&mut out);
    }
    out
}

/// `None` for anything that is not a whole number of entries (treated as a miss).
fn decode<R: Record>(bytes: &[u8]) -> Option<Vec<R>> {
    if !bytes.len().is_multiple_of(R::SIZE) {
        return None;
    }
    bytes.chunks_exact(R::SIZE).map(R::read).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_lists_roundtrip_through_bytes() {
        let ids = vec![Uuid::now_v7(), Uuid::now_v7(), Uuid::nil()];
        assert_eq!(decode(&encode(&ids)), Some(ids));
        assert_eq!(decode::<Uuid>(&[]), Some(Vec::new()), "an empty list is a hit, not a miss");
        assert_eq!(decode::<Uuid>(&[1, 2, 3]), None);
        let pairs = vec![(Uuid::now_v7(), Uuid::now_v7())];
        assert_eq!(decode(&encode(&pairs)), Some(pairs));
        assert_eq!(decode::<(Uuid, Uuid)>(&[0; 16]), None, "half a pair is a miss");
    }

    /// A reader that loaded the list before a change committed must not put
    /// it into the cache, however long it takes to get there.
    #[tokio::test]
    async fn a_slow_reader_cannot_cache_a_list_from_before_a_change() {
        let app = tokuly_testkit::TestApp::start(&["0001", "0100", "0200", "0400"], |_, _, router| router).await;
        let core = &app.core;
        let user = Uuid::now_v7();
        let before_the_change = vec![Uuid::now_v7()];
        let loaded = FRIENDS
            .get_or_load(core, user, || async {
                let snapshot = before_the_change.clone(); // read from the database...
                FRIENDS.invalidate(core, &[user]).await; // ...then a writer commits and invalidates...
                tokio::time::sleep(std::time::Duration::from_secs(6)).await; // ...while this reader stalls
                Ok(snapshot)
            })
            .await
            .unwrap();
        assert_eq!(loaded, before_the_change, "the caller still gets what it read");
        assert_eq!(FRIENDS.read(core, user).await.unwrap().bytes, None, "but the stale list is not cached");
    }
}
