//! Transaction-scoped advisory locks that order a stored notification
//! against a change that must remove it.
//!
//! The consumer stores an entry after checking that nothing forbids it (its
//! content was not withdrawn, sender and recipient are not in a block). A
//! withdrawal or a block, handled concurrently, removes such entries. Without
//! an order between the two, the check can pass before the change commits
//! and the entry be stored after the change looked for it: it would stay.
//! So the change holds a key exclusively while it writes and removes, and the
//! consumer holds the same key shared while it checks and stores.
//!
//! The registrations of one session are serialized the same way (`devices.rs`).
//!
//! Keys are derived from UUIDs. A collision only makes unrelated work wait.
use sqlx::{Postgres, Transaction};
use tokuly_core::prelude::*;

/// The key of one ID (withdrawn content).
pub(crate) fn key(id: Uuid) -> i64 {
    let bits = id.as_u128();
    (bits as i64) ^ ((bits >> 64) as i64)
}

/// The key of a pair of users, the same in both directions (blocks).
pub(crate) fn pair_key(a: Uuid, b: Uuid) -> i64 {
    let (low, high) = if a < b { (a, b) } else { (b, a) };
    key(low).rotate_left(1) ^ key(high)
}

/// Takes `key` exclusively for the rest of the transaction.
pub(crate) async fn lock(tx: &mut Transaction<'_, Postgres>, key: i64) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock($1)").bind(key).execute(&mut **tx).await?;
    Ok(())
}

/// Takes `keys` shared for the rest of the transaction.
pub(crate) async fn lock_shared(tx: &mut Transaction<'_, Postgres>, mut keys: Vec<i64>) -> AppResult<()> {
    if keys.is_empty() {
        return Ok(());
    }
    keys.sort_unstable();
    keys.dedup();
    sqlx::query("SELECT pg_advisory_xact_lock_shared(k) FROM unnest($1::bigint[]) AS k")
        .bind(&keys)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pair_has_one_key_whichever_way_round() {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        assert_eq!(pair_key(a, b), pair_key(b, a));
        assert_ne!(pair_key(a, b), pair_key(a, Uuid::now_v7()));
    }
}
