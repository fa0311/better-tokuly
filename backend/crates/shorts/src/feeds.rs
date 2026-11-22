//! The three swipe feeds.
//!
//! **For you** is ranked ([`crate::ranking`]). Ranking a viewer's whole
//! candidate pool for every page would be slow and, worse, unstable: scores
//! move and shorts arrive between two pages. So the first call ranks a
//! *batch* and stores the ordered IDs in Valkey for half an hour; the cursor
//! is the batch's token and an offset into it, and pages are read off the
//! stored list. Whatever is published or deleted meanwhile, a batch never
//! repeats or skips a short. When a batch is used up (or expired) the next
//! call ranks a new one, without the shorts the viewer has seen by then.
//! When every candidate has been seen, the pool is ranked again with the
//! seen shorts allowed and the items are flagged `repeat`: the feed never
//! runs dry while there is anything to show.
//!
//! **Following** and **friends** are chronological with a keyset cursor on
//! `(published_at, id)`, which new shorts cannot disturb.
//!
//! Visibility is checked again when a page is read: a short that was
//! deleted, hidden, made private or blocked since its batch was ranked
//! drops out of the page.
//!
//! Ranking a batch costs half a dozen queries, and a client can ask for a
//! new one with every call by sending no cursor: new batches are limited
//! per viewer (30 a minute) apart from the limit on the RPC itself.
//!
//! | Valkey key | Type | Purpose |
//! |---|---|---|
//! | `shorts:foryou:<user or "anon">:<token>` | JSON, TTL `feed_batch_ttl_seconds` | a ranked for-you batch being paged through |
use std::{collections::HashMap, time::Duration};

use serde::{Deserialize, Serialize};
use tokuly_core::{page::encode_cursor, prelude::*};
use tokuly_proto::{
    common::{PageInfo, UserSummary},
    shorts::{FeedItem, FeedReason},
};

use crate::{
    Shorts,
    access::{self, Viewer},
    hydrate,
    ranking::{self, Taste},
    shorts::{self, ShortRow, short_columns},
    signals,
};

/// Shorts per page when the client names no limit, and at most.
pub const DEFAULT_PAGE: i64 = 10;
pub const MAX_PAGE: i64 = 30;

/// One place in a ranked for-you batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BatchItem {
    id: Uuid,
    /// `FeedReason` as its number.
    reason: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    hashtag: String,
    /// The viewer had seen it when the batch was ranked.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    repeat: bool,
}

/// The for-you cursor: where in which batch the next page starts. An empty
/// token asks for a new batch.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct BatchCursor {
    #[serde(default, rename = "t")]
    token: String,
    #[serde(default, rename = "o")]
    offset: usize,
}

fn batch_key(viewer: &Viewer, token: &str) -> String {
    match viewer.user_id {
        Some(user_id) => format!("shorts:foryou:{}:{token}", user_id.simple()),
        None => format!("shorts:foryou:anon:{token}"),
    }
}

/// A token as this module issues them (it becomes part of a Valkey key).
fn is_token(token: &str) -> bool {
    token.len() == 16 && token.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// What a feed attaches to a short besides the short itself.
#[derive(Debug, Clone, Default)]
struct Placement {
    reason: i32,
    hashtag: String,
    repeat: bool,
    /// Name the friend who owns the channel.
    name_friend: bool,
}

impl Shorts {
    /// One page of the for-you feed. `who` identifies the caller for the
    /// limit on new batches: the account, or the address of a guest.
    pub async fn for_you(&self, viewer: &Viewer, who: &str, page: &Page) -> AppResult<(Vec<FeedItem>, PageInfo)> {
        let core = &self.core;
        let cursor: BatchCursor = page.decode()?.unwrap_or_default();
        let mut batch: Option<(String, Vec<BatchItem>)> = None;
        if is_token(&cursor.token) {
            let stored: Option<Vec<BatchItem>> = core.cache.get_json(&batch_key(viewer, &cursor.token)).await?;
            // A batch that expired, or that has nothing left, is simply replaced.
            batch = stored.filter(|items| cursor.offset < items.len()).map(|items| (cursor.token.clone(), items));
        }
        let (token, items, mut offset) = match batch {
            Some((token, items)) => (token, items, cursor.offset),
            None => {
                core.cache.rate_limit(&format!("shorts-foryou-batch:{who}"), 30, Duration::from_secs(60)).await?;
                let items = self.rank_batch(viewer).await?;
                if items.is_empty() {
                    return Ok((Vec::new(), PageInfo::default()));
                }
                let token = tokuly_core::ids::random_base62(16);
                let ttl = Duration::from_secs(self.cfg.feed_batch_ttl_seconds);
                core.cache.set_json(&batch_key(viewer, &token), &items, Some(ttl)).await?;
                (token, items, 0)
            }
        };

        let limit = page.limit as usize;
        let mut out: Vec<FeedItem> = Vec::with_capacity(limit);
        // Shorts that vanished since the batch was ranked leave gaps: read on.
        while out.len() < limit && offset < items.len() {
            let end = (offset + limit - out.len()).min(items.len());
            let slice = &items[offset..end];
            offset = end;
            let ids: Vec<Uuid> = slice.iter().map(|item| item.id).collect();
            let placements: HashMap<Uuid, Placement> = slice
                .iter()
                .map(|item| {
                    let friend = item.reason == FeedReason::FEED_REASON_FRIEND as i32;
                    (
                        item.id,
                        Placement {
                            reason: item.reason,
                            hashtag: item.hashtag.clone(),
                            repeat: item.repeat,
                            name_friend: friend,
                        },
                    )
                })
                .collect();
            let rows: Vec<ShortRow> = shorts::find_many(core, &ids)
                .await?
                .into_iter()
                .filter(|row| !viewer.is(row.owner_id) && access::feeds_carry(viewer, &row.facts()))
                .collect();
            let rows = self.without_hidden(viewer, rows).await?;
            out.extend(self.feed_items(viewer, &rows, &placements).await?);
        }
        let next = if offset < items.len() { BatchCursor { token, offset } } else { BatchCursor::default() };
        Ok((out, PageInfo { next_cursor: encode_cursor(&next), has_more: true, ..Default::default() }))
    }

    /// Ranks a new for-you batch for `viewer`: unseen candidates, or, when
    /// the viewer has seen them all, the same pool again as repeats.
    async fn rank_batch(&self, viewer: &Viewer) -> AppResult<Vec<BatchItem>> {
        let core = &self.core;
        let taste = Taste::load(core, viewer).await?;
        let seen: Vec<Uuid> = match viewer.user_id {
            Some(user_id) => signals::seen_ids(core, user_id).await?.into_iter().collect(),
            None => Vec::new(),
        };
        let per_source = self.cfg.feed_batch_size as i64;
        let mut repeat = false;
        let mut candidates = ranking::candidates(core, viewer, &taste, &seen, per_source).await?;
        if candidates.is_empty() && !seen.is_empty() {
            repeat = true;
            candidates = ranking::candidates(core, viewer, &taste, &[], per_source).await?;
        }
        Ok(ranking::rank(&candidates, &taste)
            .into_iter()
            .take(self.cfg.feed_batch_size)
            .map(|ranked| BatchItem {
                id: ranked.id,
                reason: ranked.reason as i32,
                hashtag: ranked.hashtag.unwrap_or_default(),
                repeat,
            })
            .collect())
    }

    /// Drops the rows the viewer marked "not interested".
    async fn without_hidden(&self, viewer: &Viewer, rows: Vec<ShortRow>) -> AppResult<Vec<ShortRow>> {
        let Some(user_id) = viewer.user_id else { return Ok(rows) };
        if rows.is_empty() {
            return Ok(rows);
        }
        let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        let hidden: Vec<Uuid> =
            sqlx::query_scalar("SELECT short_id FROM short_hidden WHERE user_id = $1 AND short_id = ANY($2)")
                .bind(user_id)
                .bind(&ids)
                .fetch_all(&self.core.db)
                .await?;
        Ok(rows.into_iter().filter(|row| !hidden.contains(&row.id)).collect())
    }

    /// Feed items for `rows`, in order, with what `placements` says about
    /// each. The owner of a channel is only ever named to their friends.
    async fn feed_items(
        &self,
        viewer: &Viewer,
        rows: &[ShortRow],
        placements: &HashMap<Uuid, Placement>,
    ) -> AppResult<Vec<FeedItem>> {
        let core = &self.core;
        let mut friend_ids: Vec<Uuid> = rows
            .iter()
            .filter(|row| placements.get(&row.id).is_some_and(|placement| placement.name_friend))
            .map(|row| row.owner_id)
            .filter(|owner_id| viewer.friends.contains(owner_id))
            .collect();
        friend_ids.sort_unstable();
        friend_ids.dedup();
        let friends: HashMap<Uuid, UserSummary> = tokuly_identity::summaries(core, &friend_ids).await?;
        let owners: HashMap<String, Uuid> = rows.iter().map(|row| (row.id.to_string(), row.owner_id)).collect();
        Ok(hydrate::shorts(core, viewer, rows)
            .await?
            .into_iter()
            .map(|short| {
                let id = Uuid::parse_str(&short.id).unwrap_or_default();
                let placement = placements.get(&id).cloned().unwrap_or_default();
                let friend = owners
                    .get(&short.id)
                    .filter(|_| placement.name_friend)
                    .and_then(|owner_id| friends.get(owner_id).cloned());
                // "From a friend" is only said while it is true: a batch
                // ranked before a friendship ended must not keep saying it.
                let stale = placement.reason == FeedReason::FEED_REASON_FRIEND as i32 && friend.is_none();
                FeedItem {
                    short: Some(short).into(),
                    reason: if stale { FeedReason::FEED_REASON_UNSPECIFIED.into() } else { placement.reason.into() },
                    hashtag: placement.hashtag,
                    friend: friend.into(),
                    repeat: placement.repeat,
                    ..Default::default()
                }
            })
            .collect())
    }

    /// The shorts `viewer` has seen, when a chronological feed is asked to
    /// leave them out.
    async fn seen_to_skip(&self, viewer: &Viewer, skip_seen: bool) -> AppResult<Vec<Uuid>> {
        match viewer.user_id {
            Some(user_id) if skip_seen => Ok(signals::seen_ids(&self.core, user_id).await?.into_iter().collect()),
            _ => Ok(Vec::new()),
        }
    }

    /// One page of the following feed: shorts of followed channels, newest first.
    pub async fn following(
        &self,
        viewer: &Viewer,
        page: &Page,
        skip_seen: bool,
    ) -> AppResult<(Vec<FeedItem>, PageInfo)> {
        let before: Option<(Time, Uuid)> = page.decode()?;
        let seen = self.seen_to_skip(viewer, skip_seen).await?;
        let mut rows: Vec<ShortRow> = sqlx::query_as(concat!(
            "SELECT ",
            short_columns!(),
            " FROM shorts s
              WHERE s.channel_id IN (SELECT f.channel_id FROM short_follows f WHERE f.user_id = $1) AND ",
            access::in_feed!(2, 3),
            " AND NOT (s.id = ANY($4))
              AND NOT EXISTS (SELECT 1 FROM short_hidden x WHERE x.user_id = $1 AND x.short_id = s.id)
              AND ($5::timestamptz IS NULL OR (s.published_at, s.id) < ($5, $6))
              ORDER BY s.published_at DESC, s.id DESC LIMIT $7"
        ))
        .bind(viewer.user_id)
        .bind(&viewer.friends)
        .bind(&viewer.blocked)
        .bind(&seen)
        .bind(before.map(|(at, _)| at))
        .bind(before.map(|(_, id)| id))
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.published_at.unwrap_or(row.created_at), row.id));
        let items = self.feed_items(viewer, &rows, &HashMap::new()).await?;
        Ok((items, info))
    }

    /// One page of the friends feed: shorts of channels owned by friends,
    /// newest first, each naming the friend.
    pub async fn friends(&self, viewer: &Viewer, page: &Page, skip_seen: bool) -> AppResult<(Vec<FeedItem>, PageInfo)> {
        let before: Option<(Time, Uuid)> = page.decode()?;
        if viewer.friends.is_empty() {
            return Ok((Vec::new(), PageInfo::default()));
        }
        let seen = self.seen_to_skip(viewer, skip_seen).await?;
        let mut rows: Vec<ShortRow> = sqlx::query_as(concat!(
            "SELECT ",
            short_columns!(),
            " FROM shorts s WHERE s.owner_id = ANY($2) AND ",
            access::in_feed!(2, 3),
            " AND NOT (s.id = ANY($4))
              AND NOT EXISTS (SELECT 1 FROM short_hidden x WHERE x.user_id = $1 AND x.short_id = s.id)
              AND ($5::timestamptz IS NULL OR (s.published_at, s.id) < ($5, $6))
              ORDER BY s.published_at DESC, s.id DESC LIMIT $7"
        ))
        .bind(viewer.user_id)
        .bind(&viewer.friends)
        .bind(&viewer.blocked)
        .bind(&seen)
        .bind(before.map(|(at, _)| at))
        .bind(before.map(|(_, id)| id))
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.published_at.unwrap_or(row.created_at), row.id));
        let placements: HashMap<Uuid, Placement> =
            rows.iter().map(|row| (row.id, Placement { name_friend: true, ..Default::default() })).collect();
        let items = self.feed_items(viewer, &rows, &placements).await?;
        Ok((items, info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursors_round_trip_and_default_to_a_new_batch() {
        let cursor = BatchCursor { token: "abcDEF0123456789".to_owned(), offset: 20 };
        let decoded: BatchCursor = tokuly_core::page::decode_cursor(&encode_cursor(&cursor)).unwrap();
        assert_eq!((decoded.token.as_str(), decoded.offset), ("abcDEF0123456789", 20));
        let fresh: BatchCursor = tokuly_core::page::decode_cursor(&encode_cursor(&BatchCursor::default())).unwrap();
        assert!(fresh.token.is_empty() && fresh.offset == 0);
        assert!(tokuly_core::page::decode_cursor::<BatchCursor>("not a cursor").is_err());
    }

    #[test]
    fn only_issued_tokens_reach_valkey() {
        assert!(is_token("abcDEF0123456789"));
        assert!(!is_token(""));
        assert!(!is_token("short"));
        assert!(!is_token("abcDEF012345678*"));
        assert!(!is_token("abcDEF0123456789:x"));
    }

    #[test]
    fn batches_are_stored_compactly() {
        let plain = BatchItem { id: Uuid::nil(), reason: 5, hashtag: String::new(), repeat: false };
        let json = serde_json::to_string(&plain).unwrap();
        assert!(!json.contains("hashtag") && !json.contains("repeat"));
        let back: BatchItem = serde_json::from_str(&json).unwrap();
        assert!(!back.repeat && back.hashtag.is_empty() && back.reason == 5);
    }
}
