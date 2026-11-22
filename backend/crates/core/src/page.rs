//! Cursor pagination helpers.
//!
//! Cursors are opaque to clients: a base64url JSON value of whatever keyset
//! the query sorts by (usually `(created_at, id)` or just an ID).
use serde::{Serialize, de::DeserializeOwned};
use tokuly_proto::common::{PageInfo, PageRequest, PageRequestView};

use crate::{
    error::{AppError, AppResult},
    keys::{b64url, b64url_decode},
};

/// A normalized page request.
#[derive(Debug, Clone)]
pub struct Page {
    pub limit: i64,
    pub cursor: Option<String>,
}

impl Page {
    pub const DEFAULT_LIMIT: i64 = 30;

    pub fn new(limit: i32, cursor: &str, default: i64, max: i64) -> Self {
        let limit = if limit <= 0 { default } else { (limit as i64).min(max) };
        Self { limit, cursor: (!cursor.is_empty()).then(|| cursor.to_owned()) }
    }

    /// From an optional request field with the standard default (30) and a maximum.
    pub fn from_view(page: Option<&PageRequestView<'_>>, max: i64) -> Self {
        match page {
            Some(p) => Self::new(p.limit, p.cursor, Self::DEFAULT_LIMIT.min(max), max),
            None => Self::new(0, "", Self::DEFAULT_LIMIT.min(max), max),
        }
    }

    pub fn from_request(page: Option<&PageRequest>, max: i64) -> Self {
        match page {
            Some(p) => Self::new(p.limit, &p.cursor, Self::DEFAULT_LIMIT.min(max), max),
            None => Self::new(0, "", Self::DEFAULT_LIMIT.min(max), max),
        }
    }

    /// Number of rows to fetch: one more than the limit, to learn whether
    /// another page exists.
    pub fn fetch(&self) -> i64 {
        self.limit + 1
    }

    /// Decodes the cursor into the query's keyset type.
    pub fn decode<T: DeserializeOwned>(&self) -> AppResult<Option<T>> {
        match &self.cursor {
            None => Ok(None),
            Some(raw) => decode_cursor(raw).map(Some),
        }
    }

    /// Trims the over-fetched row and builds the response page info. `key`
    /// extracts the keyset from the last returned row.
    pub fn finish<R, K: Serialize>(&self, rows: &mut Vec<R>, key: impl Fn(&R) -> K) -> PageInfo {
        let has_more = rows.len() as i64 > self.limit;
        if has_more {
            rows.truncate(self.limit as usize);
        }
        let next_cursor = match (has_more, rows.last()) {
            (true, Some(last)) => encode_cursor(&key(last)),
            _ => String::new(),
        };
        PageInfo { next_cursor, has_more, ..Default::default() }
    }
}

pub fn encode_cursor<T: Serialize>(value: &T) -> String {
    b64url(&serde_json::to_vec(value).expect("cursor keysets serialize"))
}

pub fn decode_cursor<T: DeserializeOwned>(raw: &str) -> AppResult<T> {
    b64url_decode(raw)
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or_else(|| AppError::invalid("INVALID_CURSOR", "invalid page cursor"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finish_trims_and_sets_cursor() {
        let page = Page::new(2, "", 30, 100);
        let mut rows = vec![1, 2, 3];
        let info = page.finish(&mut rows, |r| *r);
        assert_eq!(rows, vec![1, 2]);
        assert!(info.has_more);
        let next = Page::new(2, &info.next_cursor, 30, 100);
        assert_eq!(next.decode::<i32>().unwrap(), Some(2));
    }

    #[test]
    fn last_page_has_no_cursor() {
        let page = Page::new(5, "", 30, 100);
        let mut rows = vec![1, 2];
        let info = page.finish(&mut rows, |r| *r);
        assert!(!info.has_more && info.next_cursor.is_empty());
    }

    #[test]
    fn limits_are_clamped_and_bad_cursors_rejected() {
        assert_eq!(Page::new(0, "", 30, 100).limit, 30);
        assert_eq!(Page::new(1000, "", 30, 100).limit, 100);
        assert!(Page::new(1, "!!!", 30, 100).decode::<i32>().is_err());
    }
}
