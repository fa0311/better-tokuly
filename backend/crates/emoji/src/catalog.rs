//! The catalogue: the approved emoji, as clients and other domains use them.
//!
//! # For other domains
//!
//! Anything that shows text, names or reactions resolves `:name:` through
//! four functions, all answered from the cached catalogue (one small Valkey
//! read per call, no database query), so they can be called on every request:
//!
//! - [`exists`] — is there an emoji with this name?
//! - [`resolve`] — the images of the names found in a text;
//! - [`lookup`] — the same with the whole [`CustomEmoji`] (flags, category);
//! - [`record_use`] — a user used these emoji (feeds "recently used" and
//!   "popular").
//!
//! Names are matched without regard to case, with or without colons.
//!
//! # Versions
//!
//! The catalogue has a version that grows by one with every change to the
//! set of approved emoji or to the categories. Each approved emoji remembers
//! the version of its last change, and each emoji that left the catalogue
//! leaves a removal behind, so "what changed since version N" is a filter.
//! Changes collect in a [`Pending`] while their transaction runs and take
//! their version as the transaction's last step ([`Pending::seal`]): the
//! version counter is one row, whoever updates it holds it until commit, so
//! versions are handed out in commit order and a reader never sees version
//! N without everything that came before it.
//!
//! # Cache
//!
//! The whole catalogue is kept serialized in Valkey next to its version
//! number, and decoded once per process ([`snapshot`]). A reader compares
//! the version in Valkey with the one it holds and reloads on a difference.
//! After a change commits, [`published`] raises the version in Valkey (which
//! drops the serialized copy) and announces the new version on the realtime
//! topic [`TOPIC`], at most about once a second. A worker repeats both
//! every minute ([`reconcile`]) in case a process died between commit and
//! announcement, and takes the cache back when the database was restored to
//! an earlier version.
//!
//! Keys carry the catalogue's `epoch`, an ID created with the database, so a
//! Valkey shared by several databases (test runs, staging next to
//! production) never serves one database's catalogue to another.
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use tokuly_core::prelude::*;
use tokuly_proto::{
    common::Image,
    emoji::{CatalogChanged, Category, CustomEmoji, EmojiEvent},
    realtime::server_frame::Body,
};

use crate::{
    model::{EmojiRow, emoji_columns},
    usage::{self, Item},
    validate,
};

/// Public realtime topic on which new catalogue versions are announced.
pub const TOPIC: &str = "emoji:catalog";

/// How long the serialized catalogue stays in Valkey without being rewritten.
const DATA_TTL_SECONDS: u64 = 3600;
/// Shortest time between two announcements.
const ANNOUNCE_INTERVAL_MS: u64 = 1000;
/// How long a process trusts the popularity numbers it read.
const POPULARITY_TTL: Duration = Duration::from_secs(30);

// ───────────────────────────── For other domains ─────────────────────────────

/// Whether an approved emoji has this name.
pub async fn exists(core: &Core, name: &str) -> AppResult<bool> {
    Ok(snapshot(core).await?.get(name).is_some())
}

/// The images of the approved emoji among `names`, keyed by the name as it
/// was passed in. Names that are no emoji (or not approved) are absent.
pub async fn resolve(core: &Core, names: &[String]) -> AppResult<HashMap<String, Image>> {
    let snapshot = snapshot(core).await?;
    Ok(names
        .iter()
        .filter_map(|name| {
            let image = snapshot.get(name)?.image.as_option()?.clone();
            Some((name.clone(), image))
        })
        .collect())
}

/// Like [`resolve`], with everything the catalogue knows about each emoji.
pub async fn lookup(core: &Core, names: &[String]) -> AppResult<HashMap<String, CustomEmoji>> {
    let snapshot = snapshot(core).await?;
    Ok(names.iter().filter_map(|name| Some((name.clone(), snapshot.get(name)?.clone()))).collect())
}

/// Records that `user_id` used these emoji, once each: they move to the top
/// of the user's "recently used" and count towards "popular". Call it after
/// the message or reaction is stored. Besides names of custom emoji the list
/// may hold Unicode emoji (remembered as recently used, not counted);
/// anything else is ignored.
pub async fn record_use(core: &Core, user_id: Uuid, names: &[String]) -> AppResult<()> {
    let snapshot = snapshot(core).await?;
    let items: Vec<Item> = names.iter().filter_map(|name| Item::parse(name)).collect();
    usage::record(core, &snapshot, user_id, &items).await
}

// ───────────────────────────── Snapshot ─────────────────────────────

/// One approved emoji as the cache stores it.
#[derive(Clone, Serialize, Deserialize)]
struct StoredEntry {
    emoji: CustomEmoji,
    /// Catalogue version of its last change.
    version: i64,
}

/// The catalogue as it is serialized into Valkey.
#[derive(Serialize, Deserialize)]
struct Stored {
    version: i64,
    floor_version: i64,
    /// Ordered by name.
    entries: Vec<StoredEntry>,
    /// Category names in display order.
    categories: Vec<String>,
    /// Emoji that left the catalogue, with the version at which they did.
    removals: Vec<(Uuid, i64)>,
}

struct Entry {
    emoji: CustomEmoji,
    id: Uuid,
    version: i64,
    /// When it joined the catalogue, in microseconds.
    added_at: i64,
    /// Aliases and category in comparison form ([`validate::fold`]).
    aliases: Vec<String>,
    category: String,
}

/// What a client holding an older version needs to catch up.
#[derive(Debug, Default)]
pub struct Delta {
    pub version: i64,
    /// `upserted` is the whole catalogue.
    pub full: bool,
    pub upserted: Vec<CustomEmoji>,
    pub removed_ids: Vec<String>,
}

/// The decoded catalogue at one version. Immutable; shared by every request
/// of a process until the version moves.
pub struct Snapshot {
    pub version: i64,
    floor_version: i64,
    /// Ordered by name.
    entries: Vec<Entry>,
    by_name: HashMap<String, usize>,
    by_id: HashMap<Uuid, usize>,
    /// Indexes into `entries`, most recently added first.
    newest: Vec<usize>,
    categories: Vec<Category>,
    uncategorized_count: i32,
    removals: Vec<(Uuid, i64)>,
}

impl Snapshot {
    fn build(stored: Stored) -> Self {
        let mut counts: HashMap<&str, i32> = HashMap::new();
        for entry in &stored.entries {
            *counts.entry(entry.emoji.category.as_str()).or_default() += 1;
        }
        let uncategorized_count = counts.get("").copied().unwrap_or(0);
        let categories = stored
            .categories
            .iter()
            .map(|name| Category {
                name: name.clone(),
                emoji_count: counts.get(name.as_str()).copied().unwrap_or(0),
                ..Default::default()
            })
            .collect();

        let entries: Vec<Entry> = stored
            .entries
            .into_iter()
            .map(|stored| Entry {
                id: Uuid::parse_str(&stored.emoji.id).unwrap_or_default(),
                version: stored.version,
                added_at: stored.emoji.added_at.as_option().map(|at| from_ts(at).timestamp_micros()).unwrap_or(0),
                aliases: stored.emoji.aliases.iter().map(|alias| validate::fold(alias)).collect(),
                category: validate::fold(&stored.emoji.category),
                emoji: stored.emoji,
            })
            .collect();
        let by_name = entries.iter().enumerate().map(|(index, entry)| (entry.emoji.name.clone(), index)).collect();
        let by_id = entries.iter().enumerate().map(|(index, entry)| (entry.id, index)).collect();
        let mut newest: Vec<usize> = (0..entries.len()).collect();
        newest.sort_by(|a, b| (entries[*b].added_at, entries[*b].id).cmp(&(entries[*a].added_at, entries[*a].id)));
        Self {
            version: stored.version,
            floor_version: stored.floor_version,
            entries,
            by_name,
            by_id,
            newest,
            categories,
            uncategorized_count,
            removals: stored.removals,
        }
    }

    /// The approved emoji called `name` (any case, with or without colons).
    pub fn get(&self, name: &str) -> Option<&CustomEmoji> {
        let name = validate::strip_colons(name);
        let index = match self.by_name.get(name) {
            Some(index) => *index,
            // Names are stored lowercase; only pay for lowering when needed.
            None if name.bytes().any(|b| b.is_ascii_uppercase()) => *self.by_name.get(&name.to_ascii_lowercase())?,
            None => return None,
        };
        Some(&self.entries[index].emoji)
    }

    pub fn get_by_id(&self, id: Uuid) -> Option<&CustomEmoji> {
        self.by_id.get(&id).map(|index| &self.entries[*index].emoji)
    }

    /// Every approved emoji, ordered by name.
    pub fn emojis(&self) -> Vec<CustomEmoji> {
        self.entries.iter().map(|entry| entry.emoji.clone()).collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The categories in display order, with their emoji counts.
    pub fn categories(&self) -> Vec<Category> {
        self.categories.clone()
    }

    /// Approved emoji without a category.
    pub fn uncategorized_count(&self) -> i32 {
        self.uncategorized_count
    }

    /// The name of a category as it is stored, found without regard to case.
    pub fn category_named(&self, name: &str) -> Option<&str> {
        let wanted = name.to_lowercase();
        self.categories.iter().map(|category| category.name.as_str()).find(|name| name.to_lowercase() == wanted)
    }

    /// What changed after `since`. The whole catalogue when `since` is not a
    /// version this snapshot can compare with: zero (a client with no copy),
    /// a version from the future (another database), or one from before the
    /// oldest removal still remembered.
    pub fn changes_since(&self, since: i64) -> Delta {
        if since == self.version {
            return Delta { version: self.version, ..Default::default() };
        }
        if since <= 0 || since > self.version || since < self.floor_version {
            return Delta { version: self.version, full: true, upserted: self.emojis(), removed_ids: Vec::new() };
        }
        Delta {
            version: self.version,
            full: false,
            upserted: self
                .entries
                .iter()
                .filter(|entry| entry.version > since)
                .map(|entry| entry.emoji.clone())
                .collect(),
            removed_ids: self
                .removals
                .iter()
                .filter(|(id, version)| *version > since && !self.by_id.contains_key(id))
                .map(|(id, _)| id.to_string())
                .collect(),
        }
    }

    /// Searches names, aliases and categories. `query` must be folded
    /// ([`validate::query`]); `category` narrows to one category (`""` =
    /// emoji without one). Best matches first; among equals the more used
    /// emoji, then the shorter name. An empty query lists the category by
    /// name, or nothing when there is none.
    pub fn search(
        &self,
        query: &str,
        category: Option<&str>,
        limit: usize,
        popularity: &HashMap<Uuid, f64>,
    ) -> Vec<CustomEmoji> {
        let category = category.map(validate::fold);
        let in_category = |entry: &Entry| category.as_ref().is_none_or(|wanted| entry.category == *wanted);
        if query.is_empty() {
            if category.is_none() {
                return Vec::new();
            }
            return self
                .entries
                .iter()
                .filter(|entry| in_category(entry))
                .take(limit)
                .map(|entry| entry.emoji.clone())
                .collect();
        }
        let mut hits: Vec<(u8, f64, &Entry)> = self
            .entries
            .iter()
            .filter(|entry| in_category(entry))
            .filter_map(|entry| {
                Some((match_rank(entry, query)?, popularity.get(&entry.id).copied().unwrap_or(0.0), entry))
            })
            .collect();
        hits.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(b.1.total_cmp(&a.1))
                .then(a.2.emoji.name.len().cmp(&b.2.emoji.name.len()))
                .then_with(|| a.2.emoji.name.cmp(&b.2.emoji.name))
        });
        hits.into_iter().take(limit).map(|(_, _, entry)| entry.emoji.clone()).collect()
    }

    /// The most used emoji, most used first.
    pub fn popular(&self, limit: usize, popularity: &HashMap<Uuid, f64>) -> Vec<CustomEmoji> {
        let mut used: Vec<(f64, &Entry)> = self
            .entries
            .iter()
            .filter_map(|entry| Some((popularity.get(&entry.id).copied().filter(|score| *score > 0.0)?, entry)))
            .collect();
        used.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.emoji.name.cmp(&b.1.emoji.name)));
        used.into_iter().take(limit).map(|(_, entry)| entry.emoji.clone()).collect()
    }

    /// The emoji added most recently, newest first, starting after `after`
    /// (the `(added_at, id)` key of the last emoji of the previous page).
    /// Returns up to `limit` emoji with their keys.
    pub fn newest(&self, after: Option<(i64, Uuid)>, limit: usize) -> Vec<((i64, Uuid), CustomEmoji)> {
        self.newest
            .iter()
            .map(|index| &self.entries[*index])
            .filter(|entry| after.is_none_or(|after| (entry.added_at, entry.id) < after))
            .take(limit)
            .map(|entry| ((entry.added_at, entry.id), entry.emoji.clone()))
            .collect()
    }
}

/// How well an emoji matches a folded query; lower is better, `None` is no
/// match. Exact matches first, then prefixes, then substrings; a name beats
/// an alias at each step, and the category comes last.
fn match_rank(entry: &Entry, query: &str) -> Option<u8> {
    let name = entry.emoji.name.as_str();
    if name == query {
        Some(0)
    } else if entry.aliases.iter().any(|alias| alias == query) {
        Some(1)
    } else if name.starts_with(query) {
        Some(2)
    } else if entry.aliases.iter().any(|alias| alias.starts_with(query)) {
        Some(3)
    } else if name.contains(query) {
        Some(4)
    } else if entry.aliases.iter().any(|alias| alias.contains(query)) {
        Some(5)
    } else if entry.category.contains(query) {
        Some(6)
    } else {
        None
    }
}

// ───────────────────────────── Cache ─────────────────────────────

/// The Valkey keys of one database's emoji state.
#[derive(Debug, Clone)]
pub(crate) struct Keys {
    epoch: String,
}

/// Epochs by database URL: read once per process and database.
static EPOCHS: LazyLock<RwLock<HashMap<String, Keys>>> = LazyLock::new(Default::default);
/// Decoded snapshots by epoch.
static SNAPSHOTS: LazyLock<RwLock<HashMap<String, Arc<Snapshot>>>> = LazyLock::new(Default::default);
/// Lets one request at a time reload the catalogue; the others wait for its result.
static LOADING: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(Default::default);
/// Popularity numbers by epoch, with when they were read.
static POPULARITY: LazyLock<RwLock<HashMap<String, (Instant, Arc<HashMap<Uuid, f64>>)>>> =
    LazyLock::new(Default::default);

impl Keys {
    pub(crate) async fn of(core: &Core) -> AppResult<Self> {
        if let Some(keys) = EPOCHS.read().get(&core.cfg.database_url) {
            return Ok(keys.clone());
        }
        let epoch: Uuid = sqlx::query_scalar("SELECT epoch FROM emoji_catalog").fetch_one(&core.db).await?;
        let keys = Self { epoch: epoch.simple().to_string() };
        EPOCHS.write().insert(core.cfg.database_url.clone(), keys.clone());
        Ok(keys)
    }

    fn version(&self) -> String {
        format!("emoji:{}:catalog:version", self.epoch)
    }

    fn data(&self) -> String {
        format!("emoji:{}:catalog:data", self.epoch)
    }

    /// The last version announced on the realtime topic.
    fn announced(&self) -> String {
        format!("emoji:{}:catalog:announced", self.epoch)
    }

    /// Exists for [`ANNOUNCE_INTERVAL_MS`] after an announcement.
    fn announce_gate(&self) -> String {
        format!("emoji:{}:catalog:gate", self.epoch)
    }

    /// Use counters waiting for the usage worker (a hash: emoji ID → uses).
    pub(crate) fn uses(&self) -> String {
        format!("emoji:{}:uses", self.epoch)
    }

    /// The counters the usage worker is writing to the database right now.
    pub(crate) fn uses_draining(&self) -> String {
        format!("emoji:{}:uses:draining", self.epoch)
    }

    /// One user's counted uses of one day (a hash: emoji ID → uses).
    pub(crate) fn user_uses(&self, day: &str, user_id: Uuid) -> String {
        format!("emoji:{}:uses:{day}:{}", self.epoch, user_id.simple())
    }
}

/// Stores a catalogue read from the database unless Valkey already knows of
/// a newer version (a change committed while this one was being read).
static STORE: LazyLock<redis::Script> = LazyLock::new(|| {
    redis::Script::new(
        "local known = tonumber(redis.call('GET', KEYS[1]) or '-1')
         if known > tonumber(ARGV[1]) then return 0 end
         redis.call('SET', KEYS[1], ARGV[1])
         redis.call('SET', KEYS[2], ARGV[2], 'EX', ARGV[3])
         return 1",
    )
});

/// Raises the version and drops the serialized catalogue of older versions.
static RAISE: LazyLock<redis::Script> = LazyLock::new(|| {
    redis::Script::new(
        "local known = tonumber(redis.call('GET', KEYS[1]) or '-1')
         if known >= tonumber(ARGV[1]) then return 0 end
         redis.call('SET', KEYS[1], ARGV[1])
         redis.call('DEL', KEYS[2])
         return 1",
    )
});

/// Takes the version back to the database's when Valkey is ahead of it,
/// which only happens when the database was restored to an earlier state.
/// The serialized catalogue goes, and so does the memory of having announced
/// versions that no longer exist.
static LOWER: LazyLock<redis::Script> = LazyLock::new(|| {
    redis::Script::new(
        "local known = tonumber(redis.call('GET', KEYS[1]) or '-1')
         if known <= tonumber(ARGV[1]) then return 0 end
         redis.call('SET', KEYS[1], ARGV[1])
         redis.call('DEL', KEYS[2])
         if tonumber(redis.call('GET', KEYS[3]) or '0') > tonumber(ARGV[1]) then
             redis.call('SET', KEYS[3], ARGV[1])
         end
         return 1",
    )
});

/// Claims the right to announce the current version. Returns the version to
/// announce, 0 when it was announced already, or minus the milliseconds to
/// wait when another announcement was claimed less than an interval ago.
///
/// Claiming does not record the announcement; [`ANNOUNCED`] does, once the
/// push is out. A claimer that stops in between loses nothing: after the
/// interval the next one finds the version still unannounced.
static CLAIM_ANNOUNCEMENT: LazyLock<redis::Script> = LazyLock::new(|| {
    redis::Script::new(
        "local version = tonumber(redis.call('GET', KEYS[1]) or '0')
         local announced = tonumber(redis.call('GET', KEYS[2]) or '0')
         if announced >= version then return 0 end
         local wait = redis.call('PTTL', KEYS[3])
         if wait > 0 then return -wait end
         redis.call('SET', KEYS[3], '1', 'PX', ARGV[1])
         return version",
    )
});

/// Records that a version was announced.
static ANNOUNCED: LazyLock<redis::Script> = LazyLock::new(|| {
    redis::Script::new(
        "local announced = tonumber(redis.call('GET', KEYS[1]) or '0')
         if announced < tonumber(ARGV[1]) then redis.call('SET', KEYS[1], ARGV[1]) end
         return 1",
    )
});

fn remembered(keys: &Keys, version: Option<i64>) -> Option<Arc<Snapshot>> {
    let version = version?;
    SNAPSHOTS.read().get(&keys.epoch).filter(|snapshot| snapshot.version == version).cloned()
}

fn remember(keys: &Keys, snapshot: Snapshot) -> Arc<Snapshot> {
    let snapshot = Arc::new(snapshot);
    SNAPSHOTS.write().insert(keys.epoch.clone(), snapshot.clone());
    snapshot
}

async fn cached_version(core: &Core, keys: &Keys) -> AppResult<Option<i64>> {
    Ok(redis::cmd("GET").arg(keys.version()).query_async(&mut core.cache.conn()).await?)
}

/// The catalogue at its current version.
pub async fn snapshot(core: &Core) -> AppResult<Arc<Snapshot>> {
    let keys = Keys::of(core).await?;
    if let Some(snapshot) = remembered(&keys, cached_version(core, &keys).await?) {
        return Ok(snapshot);
    }

    let _loading = LOADING.lock().await;
    let version = cached_version(core, &keys).await?;
    if let Some(snapshot) = remembered(&keys, version) {
        return Ok(snapshot);
    }
    let mut conn = core.cache.conn();
    if let Some(version) = version {
        let data: Option<Vec<u8>> = redis::cmd("GET").arg(keys.data()).query_async(&mut conn).await?;
        let stored = data.and_then(|bytes| serde_json::from_slice::<Stored>(&bytes).ok());
        if let Some(stored) = stored.filter(|stored| stored.version == version) {
            return Ok(remember(&keys, Snapshot::build(stored)));
        }
    }

    let stored = load(core).await?;
    let data = serde_json::to_vec(&stored).map_err(|e| AppError::internal(anyhow::anyhow!("catalogue encode: {e}")))?;
    STORE
        .key(keys.version())
        .key(keys.data())
        .arg(stored.version)
        .arg(data)
        .arg(DATA_TTL_SECONDS)
        .invoke_async::<i64>(&mut conn)
        .await?;
    Ok(remember(&keys, Snapshot::build(stored)))
}

/// Reads the catalogue from the database, as of one moment.
async fn load(core: &Core) -> AppResult<Stored> {
    let mut tx = core.db.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY").execute(&mut *tx).await?;
    let (version, floor_version): (i64, i64) =
        sqlx::query_as("SELECT version, floor_version FROM emoji_catalog").fetch_one(&mut *tx).await?;
    let rows: Vec<EmojiRow> =
        sqlx::query_as(concat!("SELECT ", emoji_columns!(), " FROM emojis e WHERE e.state = 2 ORDER BY e.name"))
            .fetch_all(&mut *tx)
            .await?;
    let categories: Vec<String> =
        sqlx::query_scalar("SELECT name FROM emoji_categories ORDER BY position, name").fetch_all(&mut *tx).await?;
    let removals: Vec<(Uuid, i64)> =
        sqlx::query_as("SELECT emoji_id, version FROM emoji_catalog_removals ORDER BY version")
            .fetch_all(&mut *tx)
            .await?;
    tx.commit().await?;
    let entries = rows.iter().map(|row| StoredEntry { emoji: row.to_proto(), version: row.catalog_version }).collect();
    Ok(Stored { version, floor_version, entries, categories, removals })
}

/// Recent-use scores of the emoji that have one, for ranking.
pub(crate) async fn popularity(core: &Core) -> AppResult<Arc<HashMap<Uuid, f64>>> {
    let keys = Keys::of(core).await?;
    if let Some((read_at, scores)) = POPULARITY.read().get(&keys.epoch)
        && read_at.elapsed() < POPULARITY_TTL
    {
        return Ok(scores.clone());
    }
    let rows: Vec<(Uuid, f64)> =
        sqlx::query_as("SELECT emoji_id, popularity FROM emoji_stats WHERE popularity > 0").fetch_all(&core.db).await?;
    let scores = Arc::new(rows.into_iter().collect::<HashMap<_, _>>());
    POPULARITY.write().insert(keys.epoch, (Instant::now(), scores.clone()));
    Ok(scores)
}

/// Makes this process read the popularity numbers again (they were just recomputed).
pub(crate) fn forget_popularity(keys: &Keys) {
    POPULARITY.write().remove(&keys.epoch);
}

// ───────────────────────────── Changes ─────────────────────────────

/// Changes to the catalogue collected while a transaction runs.
#[derive(Debug, Default)]
pub struct Pending {
    upserted: Vec<Uuid>,
    removed: Vec<Uuid>,
    categories: bool,
}

impl Pending {
    /// An approved emoji was added or changed.
    pub fn upsert(&mut self, emoji_id: Uuid) {
        self.upserted.push(emoji_id);
    }

    /// An emoji left the catalogue.
    pub fn remove(&mut self, emoji_id: Uuid) {
        self.removed.push(emoji_id);
    }

    /// The list of categories changed.
    pub fn categories_changed(&mut self) {
        self.categories = true;
    }

    pub fn is_empty(&self) -> bool {
        self.upserted.is_empty() && self.removed.is_empty() && !self.categories
    }

    /// Gives the collected changes the next catalogue version. Must be the
    /// last thing the transaction does before it commits (see the module
    /// documentation). Returns the version, or `None` when nothing changed.
    pub async fn seal(&self, tx: &mut Transaction<'_, Postgres>) -> AppResult<Option<i64>> {
        if self.is_empty() {
            return Ok(None);
        }
        let version: i64 =
            sqlx::query_scalar("UPDATE emoji_catalog SET version = version + 1, updated_at = now() RETURNING version")
                .fetch_one(&mut **tx)
                .await?;
        if !self.upserted.is_empty() {
            sqlx::query("UPDATE emojis SET catalog_version = $1 WHERE id = ANY($2) AND state = 2")
                .bind(version)
                .bind(&self.upserted)
                .execute(&mut **tx)
                .await?;
            sqlx::query("DELETE FROM emoji_catalog_removals WHERE emoji_id = ANY($1)")
                .bind(&self.upserted)
                .execute(&mut **tx)
                .await?;
        }
        if !self.removed.is_empty() {
            sqlx::query(
                "INSERT INTO emoji_catalog_removals (emoji_id, version)
                 SELECT DISTINCT id, $1 FROM unnest($2::uuid[]) AS gone (id)
                 ON CONFLICT (emoji_id) DO UPDATE SET version = EXCLUDED.version, removed_at = now()",
            )
            .bind(version)
            .bind(&self.removed)
            .execute(&mut **tx)
            .await?;
        }
        Ok(Some(version))
    }
}

/// Makes a committed change visible: the cache moves to `version` and
/// connected clients are told. Never fails the caller — the change is
/// committed, and the worker's [`reconcile`] repeats whatever did not happen.
pub(crate) async fn published(core: &Core, version: i64) {
    let outcome: AppResult<()> = async {
        let keys = Keys::of(core).await?;
        raise(core, &keys, version).await?;
        announce(core, &keys).await
    }
    .await;
    if let Err(e) = outcome {
        tracing::warn!(version, error = %e, "catalogue change was not announced; the worker will catch up");
    }
}

async fn raise(core: &Core, keys: &Keys, version: i64) -> AppResult<bool> {
    let raised: i64 =
        RAISE.key(keys.version()).key(keys.data()).arg(version).invoke_async(&mut core.cache.conn()).await?;
    Ok(raised == 1)
}

async fn claim_announcement(core: &Core, keys: &Keys) -> AppResult<i64> {
    Ok(CLAIM_ANNOUNCEMENT
        .key(keys.version())
        .key(keys.announced())
        .key(keys.announce_gate())
        .arg(ANNOUNCE_INTERVAL_MS)
        .invoke_async(&mut core.cache.conn())
        .await?)
}

/// Pushes a version to the topic and records that it is out.
async fn push(core: &Core, keys: &Keys, version: i64) -> AppResult<()> {
    let event =
        EmojiEvent { event: Some(CatalogChanged { version, ..Default::default() }.into()), ..Default::default() };
    core.rt.to_topic(TOPIC, Body::Emoji(Box::new(event))).await?;
    ANNOUNCED.key(keys.announced()).arg(version).invoke_async::<i64>(&mut core.cache.conn()).await?;
    Ok(())
}

/// Announces the current version unless that already happened. When the
/// last announcement is less than a second old, the announcement is left
/// to a task that waits its turn, so a burst of changes (an import) reaches
/// clients as one push per second, always ending with the latest version.
async fn announce(core: &Core, keys: &Keys) -> AppResult<()> {
    let claimed = claim_announcement(core, keys).await?;
    if claimed > 0 {
        push(core, keys, claimed).await?;
    } else if claimed < 0 {
        let (core, keys) = (core.clone(), keys.clone());
        tokio::spawn(async move {
            let mut wait_ms = claimed.unsigned_abs();
            // Each round waits out one announcement by somebody else; a
            // handful is plenty, and the worker's reconcile covers the rest.
            for _ in 0..10 {
                tokio::select! {
                    _ = core.shutdown.cancelled() => return,
                    _ = tokio::time::sleep(Duration::from_millis(wait_ms + 20)) => {}
                }
                let announced = match claim_announcement(&core, &keys).await {
                    Ok(version) if version > 0 => push(&core, &keys, version).await,
                    Ok(wait) if wait < 0 => {
                        wait_ms = wait.unsigned_abs();
                        continue;
                    }
                    Ok(_) => Ok(()),
                    Err(error) => Err(error),
                };
                if let Err(e) = announced {
                    tracing::warn!(error = %e, "catalogue announcement failed; the worker will catch up");
                }
                return;
            }
        });
    }
    Ok(())
}

/// Brings the cache and the announcements in line with the database. Run
/// periodically by the worker: it repairs what [`published`] could not do
/// (a process that stopped right after committing, a Valkey hiccup).
pub(crate) async fn reconcile(core: &Core) -> AppResult<()> {
    let keys = Keys::of(core).await?;
    // The cache first, the database second: a version only reaches the
    // cache after it was committed, so the database can only be behind what
    // was just read from the cache if it was restored to an earlier state.
    let cached = cached_version(core, &keys).await?;
    let version: i64 = sqlx::query_scalar("SELECT version FROM emoji_catalog").fetch_one(&core.db).await?;
    if cached.is_some_and(|cached| cached > version) {
        LOWER
            .key(keys.version())
            .key(keys.data())
            .key(keys.announced())
            .arg(version)
            .invoke_async::<i64>(&mut core.cache.conn())
            .await?;
        tracing::warn!(version, ?cached, "catalogue cache was ahead of the database (restored?); reset");
    } else if raise(core, &keys, version).await? {
        tracing::info!(version, "catalogue cache was behind the database");
    }
    announce(core, &keys).await
}

/// Forgets removals older than `retention_days` and moves the floor: clients
/// holding a version from before it get the whole catalogue instead of a
/// difference. Returns how many removals were forgotten.
pub(crate) async fn prune_removals(core: &Core, retention_days: i64) -> AppResult<u64> {
    let pruned: i64 = sqlx::query_scalar(
        "WITH gone AS (
             DELETE FROM emoji_catalog_removals
             WHERE removed_at < now() - make_interval(days => $1::int)
             RETURNING version
         ), floor AS (
             UPDATE emoji_catalog
             SET floor_version = GREATEST(floor_version, (SELECT max(version) FROM gone))
             WHERE EXISTS (SELECT 1 FROM gone)
         )
         SELECT count(*) FROM gone",
    )
    .bind(retention_days)
    .fetch_one(&core.db)
    .await?;
    Ok(pruned as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, version: i64, category: &str, aliases: &[&str], added_at: i64) -> StoredEntry {
        StoredEntry {
            emoji: CustomEmoji {
                id: Uuid::now_v7().to_string(),
                name: name.to_owned(),
                category: category.to_owned(),
                aliases: aliases.iter().map(|a| a.to_string()).collect(),
                added_at: ts(chrono::DateTime::from_timestamp(added_at, 0).unwrap()),
                ..Default::default()
            },
            version,
        }
    }

    fn sample() -> Snapshot {
        Snapshot::build(Stored {
            version: 9,
            floor_version: 3,
            entries: vec![
                entry("blobcat", 2, "どうぶつ", &["cat", "ネコ"], 100),
                entry("cat", 5, "どうぶつ", &["neko"], 300),
                entry("catjam", 7, "", &["dance"], 200),
                entry("copycat", 4, "Memes", &[], 50),
                entry("doge", 9, "Memes", &["category"], 400),
            ],
            categories: vec!["Memes".into(), "どうぶつ".into(), "Empty".into()],
            removals: vec![(Uuid::now_v7(), 4), (Uuid::now_v7(), 8)],
        })
    }

    fn names(emojis: &[CustomEmoji]) -> Vec<&str> {
        emojis.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn names_resolve_without_regard_to_case_or_colons() {
        let snapshot = sample();
        assert!(snapshot.get("blobcat").is_some());
        assert!(snapshot.get(":BlobCat:").is_some());
        assert!(snapshot.get("blob").is_none());
        assert!(snapshot.get("").is_none());
        let id = Uuid::parse_str(&snapshot.get("doge").unwrap().id).unwrap();
        assert_eq!(snapshot.get_by_id(id).unwrap().name, "doge");
    }

    #[test]
    fn categories_keep_their_order_and_count_their_emoji() {
        let snapshot = sample();
        let categories: Vec<(String, i32)> =
            snapshot.categories().into_iter().map(|c| (c.name, c.emoji_count)).collect();
        assert_eq!(categories, [("Memes".to_owned(), 2), ("どうぶつ".to_owned(), 2), ("Empty".to_owned(), 0)]);
        assert_eq!(snapshot.uncategorized_count(), 1);
        assert_eq!(snapshot.category_named("memes"), Some("Memes"));
        assert_eq!(snapshot.category_named("nope"), None);
    }

    #[test]
    fn search_puts_exact_then_prefix_then_substring_matches_first() {
        let snapshot = sample();
        let none = HashMap::new();
        // "cat": exact name, exact alias, name prefix, then substring; the
        // category "category" alias of doge is a prefix match on an alias.
        assert_eq!(names(&snapshot.search("cat", None, 10, &none)), ["cat", "blobcat", "catjam", "doge", "copycat"]);
        // Katakana aliases are found by hiragana, and names by any case.
        assert_eq!(names(&snapshot.search(&validate::query("ねこ").unwrap(), None, 10, &none)), ["blobcat"]);
        assert_eq!(names(&snapshot.search(&validate::query(":CatJam:").unwrap(), None, 10, &none)), ["catjam"]);
        // A category name is the weakest match; equals go by length of name.
        assert_eq!(names(&snapshot.search("memes", None, 10, &none)), ["doge", "copycat"]);
        assert!(snapshot.search("zzz", None, 10, &none).is_empty());
        assert_eq!(snapshot.search("cat", None, 2, &none).len(), 2);
    }

    #[test]
    fn search_breaks_ties_by_use() {
        let snapshot = sample();
        let doge = Uuid::parse_str(&snapshot.get("doge").unwrap().id).unwrap();
        let copycat = Uuid::parse_str(&snapshot.get("copycat").unwrap().id).unwrap();
        let popularity = HashMap::from([(doge, 1.0), (copycat, 5.0)]);
        // Both match "memes" through their category; the more used one leads.
        assert_eq!(names(&snapshot.search("memes", None, 10, &popularity)), ["copycat", "doge"]);
        let popularity = HashMap::from([(doge, 9.0), (copycat, 5.0)]);
        assert_eq!(names(&snapshot.search("memes", None, 10, &popularity)), ["doge", "copycat"]);
        // Use never lifts a weaker match over a better one.
        assert_eq!(names(&snapshot.search("cat", None, 1, &popularity)), ["cat"]);
    }

    #[test]
    fn search_can_be_narrowed_to_a_category() {
        let snapshot = sample();
        let none = HashMap::new();
        assert_eq!(names(&snapshot.search("cat", Some("memes"), 10, &none)), ["doge", "copycat"]);
        assert_eq!(names(&snapshot.search("", Some("どうぶつ"), 10, &none)), ["blobcat", "cat"]);
        assert_eq!(names(&snapshot.search("", Some(""), 10, &none)), ["catjam"]);
        assert!(snapshot.search("", None, 10, &none).is_empty());
    }

    #[test]
    fn popular_lists_only_used_emoji_most_used_first() {
        let snapshot = sample();
        let id = |name: &str| Uuid::parse_str(&snapshot.get(name).unwrap().id).unwrap();
        let popularity = HashMap::from([(id("cat"), 2.0), (id("doge"), 7.5), (id("catjam"), 0.0)]);
        assert_eq!(names(&snapshot.popular(10, &popularity)), ["doge", "cat"]);
        assert_eq!(names(&snapshot.popular(1, &popularity)), ["doge"]);
    }

    #[test]
    fn newest_pages_from_the_latest_addition() {
        let snapshot = sample();
        let first = snapshot.newest(None, 2);
        assert_eq!(first.iter().map(|(_, e)| e.name.as_str()).collect::<Vec<_>>(), ["doge", "cat"]);
        let rest = snapshot.newest(Some(first[1].0), 10);
        assert_eq!(rest.iter().map(|(_, e)| e.name.as_str()).collect::<Vec<_>>(), ["catjam", "blobcat", "copycat"]);
    }

    #[test]
    fn changes_are_a_difference_or_the_whole_catalogue() {
        let snapshot = sample();
        // Up to date.
        let same = snapshot.changes_since(9);
        assert!(!same.full && same.upserted.is_empty() && same.removed_ids.is_empty());
        // A difference: what changed and what left after version 4.
        let delta = snapshot.changes_since(4);
        assert!(!delta.full);
        assert_eq!(names(&delta.upserted), ["cat", "catjam", "doge"]);
        assert_eq!(delta.removed_ids.len(), 1);
        assert_eq!(delta.version, 9);
        // The floor itself can still be answered; below it cannot.
        assert!(!snapshot.changes_since(3).full);
        assert_eq!(snapshot.changes_since(3).removed_ids.len(), 2);
        for unknown in [0, 2, 10, -1] {
            let full = snapshot.changes_since(unknown);
            assert!(full.full && full.upserted.len() == 5 && full.removed_ids.is_empty(), "since {unknown}");
        }
    }

    #[test]
    fn an_emoji_that_came_back_is_not_reported_as_removed() {
        let mut stored = Stored {
            version: 6,
            floor_version: 0,
            entries: vec![entry("cat", 6, "", &[], 1)],
            categories: vec![],
            removals: vec![],
        };
        let id = Uuid::parse_str(&stored.entries[0].emoji.id).unwrap();
        // A stale removal of an emoji that is in the catalogue again.
        stored.removals.push((id, 5));
        let delta = Snapshot::build(stored).changes_since(4);
        assert_eq!(names(&delta.upserted), ["cat"]);
        assert!(delta.removed_ids.is_empty());
    }

    #[test]
    fn the_stored_form_survives_a_round_trip() {
        let stored = Stored {
            version: 3,
            floor_version: 1,
            entries: vec![entry("blobcat", 2, "どうぶつ", &["ねこ"], 100)],
            categories: vec!["どうぶつ".into()],
            removals: vec![(Uuid::now_v7(), 3)],
        };
        let bytes = serde_json::to_vec(&stored).unwrap();
        let back: Stored = serde_json::from_slice(&bytes).unwrap();
        assert_eq!((back.version, back.floor_version, back.removals.len()), (3, 1, 1));
        assert_eq!(back.entries[0].emoji, stored.entries[0].emoji);
        assert_eq!(back.entries[0].version, 2);
    }
}
