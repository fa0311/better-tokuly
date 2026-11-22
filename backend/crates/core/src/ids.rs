//! Identifiers: UUIDv7 primary keys plus short URL-safe IDs for shareable content.
use rand::RngExt as _;
use uuid::Uuid;

use crate::error::{AppError, AppResult};

/// A new time-ordered ID (UUIDv7).
pub fn new_id() -> Uuid {
    Uuid::now_v7()
}

/// Parses an ID received from a client.
pub fn parse_id(value: &str) -> AppResult<Uuid> {
    Uuid::parse_str(value)
        .map_err(|_| AppError::invalid("INVALID_ID", format!("invalid id: {value:?}")))
}

/// Parses an optional ID; empty string means "not set".
pub fn parse_opt_id(value: &str) -> AppResult<Option<Uuid>> {
    if value.is_empty() {
        Ok(None)
    } else {
        parse_id(value).map(Some)
    }
}

pub fn parse_ids<S: AsRef<str>>(values: &[S]) -> AppResult<Vec<Uuid>> {
    values.iter().map(|v| parse_id(v.as_ref())).collect()
}

const BASE62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Random base62 string, e.g. for tokens and codes.
pub fn random_base62(len: usize) -> String {
    let mut rng = rand::rng();
    (0..len)
        .map(|_| BASE62[rng.random_range(0..BASE62.len())] as char)
        .collect()
}

/// An 11-character public ID for shareable content (~65 bits of entropy).
pub fn new_short_id() -> String {
    random_base62(11)
}

/// A reference that is either a canonical UUID or a short public ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ref {
    Id(Uuid),
    Short(String),
}

impl Ref {
    pub fn parse(value: &str) -> AppResult<Self> {
        if let Ok(id) = Uuid::parse_str(value) {
            return Ok(Self::Id(id));
        }
        let ok = (6..=32).contains(&value.len())
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if ok {
            Ok(Self::Short(value.to_owned()))
        } else {
            Err(AppError::invalid(
                "INVALID_ID",
                format!("invalid id: {value:?}"),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_ids_are_base62() {
        let id = new_short_id();
        assert_eq!(id.len(), 11);
        assert!(id.bytes().all(|b| b.is_ascii_alphanumeric()));
    }

    #[test]
    fn refs_parse_both_forms() {
        let id = new_id();
        assert_eq!(Ref::parse(&id.to_string()).unwrap(), Ref::Id(id));
        assert_eq!(
            Ref::parse("dQw4w9WgXcQ").unwrap(),
            Ref::Short("dQw4w9WgXcQ".into())
        );
        assert!(Ref::parse("no spaces!").is_err());
    }
}
