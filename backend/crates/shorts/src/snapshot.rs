//! Paging through a list whose order keeps changing.
//!
//! "Top shorts of a hashtag" is ordered by score and "most viewed shorts of
//! a channel" by views; both move while someone pages. A keyset cursor on
//! such a value skips the short that just rose past it and repeats the one
//! that just fell behind it. So these lists are paged on a *snapshot*: the
//! ordered IDs are computed once, kept in Valkey for a few minutes under a
//! key of the list itself (one per hashtag or channel, shared by every
//! reader, so the memory is bounded by the number of lists), and the cursor
//! is an offset into it. Who may see which short is decided when a page is
//! read, per viewer.
//!
//! A cursor that outlives its snapshot reads on at the same offset in the
//! next one.
use std::{future::Future, time::Duration};

use serde::{Deserialize, Serialize};
use tokuly_core::{page::encode_cursor, prelude::*};
use tokuly_proto::common::PageInfo;

use crate::shorts::{self, ShortRow};

/// Entries a snapshot holds: how deep such a list can be paged.
pub const SNAPSHOT_SIZE: i64 = 1000;
/// How long a snapshot is read from before the list is computed again.
const SNAPSHOT_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Snapshot {
    token: String,
    ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SnapshotCursor {
    #[serde(default, rename = "t")]
    token: String,
    #[serde(default, rename = "o")]
    offset: usize,
}

/// One page of the list stored under `key`. `build` computes the ordered
/// IDs (at most [`SNAPSHOT_SIZE`]) when there is no snapshot; `keep` says
/// which of the shorts this reader may see.
pub async fn read<B, Fut>(
    core: &Core,
    key: &str,
    page: &Page,
    build: B,
    keep: impl Fn(&ShortRow) -> bool,
) -> AppResult<(Vec<ShortRow>, PageInfo)>
where
    B: FnOnce() -> Fut,
    Fut: Future<Output = AppResult<Vec<Uuid>>>,
{
    let cursor: SnapshotCursor = page.decode()?.unwrap_or_default();
    let stored: Option<Snapshot> = core.cache.get_json(key).await?;
    let snapshot = match stored {
        Some(snapshot) => snapshot,
        None => {
            let snapshot = Snapshot { token: tokuly_core::ids::random_base62(12), ids: build().await? };
            core.cache.set_json(key, &snapshot, Some(SNAPSHOT_TTL)).await?;
            snapshot
        }
    };
    let limit = page.limit as usize;
    let mut offset = cursor.offset.min(snapshot.ids.len());
    let mut rows: Vec<ShortRow> = Vec::with_capacity(limit);
    // Shorts the reader may not see leave gaps: read on.
    while rows.len() < limit && offset < snapshot.ids.len() {
        let end = (offset + limit - rows.len()).min(snapshot.ids.len());
        let found = shorts::find_many(core, &snapshot.ids[offset..end]).await?;
        offset = end;
        rows.extend(found.into_iter().filter(|row| keep(row)));
    }
    let has_more = offset < snapshot.ids.len();
    let next_cursor =
        if has_more { encode_cursor(&SnapshotCursor { token: snapshot.token, offset }) } else { String::new() };
    Ok((rows, PageInfo { next_cursor, has_more, ..Default::default() }))
}

/// Forgets the snapshot under `key`, so the next reader computes the list anew.
pub async fn forget(core: &Core, key: &str) -> AppResult<()> {
    core.cache.del(key).await
}
