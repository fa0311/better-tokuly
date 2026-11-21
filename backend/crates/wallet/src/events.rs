//! Domain events of the wallet (see `docs/events/wallet.md`).
//!
//! The only event is `wallet.balance.changed`, emitted by the ledger once per
//! user whose balance an entry changed, in the transaction that changed it.
pub use crate::ledger::BalanceChanged;

/// Topic of [`BalanceChanged`].
pub const BALANCE_CHANGED: &str = "wallet.balance.changed";

/// Topic identity emits when an account is erased; payload `{ "user_id": … }`.
pub(crate) const USER_DELETED: &str = "identity.user.deleted";
