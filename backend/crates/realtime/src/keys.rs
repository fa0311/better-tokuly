//! Valkey keys owned by the gateway (documented in `docs/events/realtime.md`).
//!
//! The online markers and topic audience reports themselves belong to
//! `tokuly_core::presence`; these are the gateway's own bookkeeping around
//! them.
use uuid::Uuid;

/// Prefix of the per-user sorted sets of foreground leases
/// (member = gateway instance, score = lease expiry in ms).
pub const FOREGROUND_PREFIX: &str = "rt:fg:u:";

/// Sorted set of users holding a foreground lease (score = latest expiry in
/// ms): the index gateways scan for leases that ran out.
pub const FOREGROUND_LEASES: &str = "rt:fg:leases";

/// Sorted set of certificate hashes (hex SHA-256) currently or soon presented
/// by WebTransport listeners (score = until when the entry is valid, in ms).
pub const CERT_HASHES: &str = "rt:wt:certs";

/// The foreground leases of one user.
pub fn foreground(user_id: Uuid) -> String {
    format!("{FOREGROUND_PREFIX}{}", user_id.simple())
}
