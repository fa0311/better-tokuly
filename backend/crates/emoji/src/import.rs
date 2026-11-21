//! Importing emoji packs (see [`crate::pack`] for the format).
//!
//! ```text
//! StartImport ─▶ emoji_imports (QUEUED), job `emoji.import`
//!                       │
//!   run_import ─────────▼──────────────────────────────────────────────────
//!     download the archive, read meta.json
//!     plan every entry (name, clash policy, file checks)  ─▶ emoji_import_entries
//!     for each accepted entry: unpack its image and hand it to the media
//!       domain as an upload (`create_asset_from_file`)     ─▶ entry.asset_id
//!                       │
//!   media processes the image, emits media.asset.ready / failed
//!                       │
//!   settle_entry ───────▼──────────────────────────────────────────────────
//!     READY  ─▶ the emoji is created, approved        ─▶ IMPORTED / RENAMED
//!     FAILED ─▶ the entry fails with media's reason   ─▶ FAILED
//!     the import is COMPLETED when its last entry is settled
//! ```
//!
//! Pictures become emoji when the media domain is done with them, not while
//! the job runs: the job only reads the pack. An entry is settled by the
//! event of its upload, by the job's last step, or by the worker's sweep —
//! whichever comes first; settling is idempotent. A job that is interrupted
//! picks up where it stopped: the plan is stored once, and entries that
//! already have their upload are skipped.
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Arc,
};

use connectrpc::ErrorCode;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokuly_core::{prelude::*, storage::Bucket};
use tokuly_media::{assets, create_asset_from_file};
use tokuly_proto::{
    common::PageInfo,
    emoji::{EmojiEvent, Import, ImportEntry},
    media::{AssetPurpose, AssetStatus},
    realtime::server_frame::Body,
};

use crate::{
    Emoji, categories,
    emojis::{self, Admission, Effects, NewEmoji, Tx},
    events::{AddedVia, ImportJob, JOB_IMPORT},
    model::{PICTURE_PURPOSE, Picture},
    pack::{Archive, PackEntry, PackError},
    validate,
};

/// `emoji_imports.status` (`tokuly.emoji.v1.ImportStatus`).
mod status {
    pub const QUEUED: i16 = 1;
    pub const RUNNING: i16 = 2;
    pub const COMPLETED: i16 = 3;
    pub const FAILED: i16 = 4;
}

/// `emoji_import_entries.status` (`tokuly.emoji.v1.ImportEntryStatus`).
mod entry_status {
    pub const PROCESSING: i16 = 1;
    pub const IMPORTED: i16 = 2;
    pub const RENAMED: i16 = 3;
    pub const SKIPPED: i16 = 4;
    pub const FAILED: i16 = 5;
}

const _: () = {
    use tokuly_proto::emoji::{ImportConflictPolicy, ImportEntryStatus, ImportStatus};
    // SQL in this file compares these columns with literals.
    assert!(ImportStatus::IMPORT_STATUS_QUEUED as i32 == status::QUEUED as i32);
    assert!(ImportStatus::IMPORT_STATUS_RUNNING as i32 == status::RUNNING as i32);
    assert!(ImportStatus::IMPORT_STATUS_COMPLETED as i32 == status::COMPLETED as i32);
    assert!(ImportStatus::IMPORT_STATUS_FAILED as i32 == status::FAILED as i32);
    assert!(ImportEntryStatus::IMPORT_ENTRY_STATUS_PROCESSING as i32 == entry_status::PROCESSING as i32);
    assert!(ImportEntryStatus::IMPORT_ENTRY_STATUS_IMPORTED as i32 == entry_status::IMPORTED as i32);
    assert!(ImportEntryStatus::IMPORT_ENTRY_STATUS_RENAMED as i32 == entry_status::RENAMED as i32);
    assert!(ImportEntryStatus::IMPORT_ENTRY_STATUS_SKIPPED as i32 == entry_status::SKIPPED as i32);
    assert!(ImportEntryStatus::IMPORT_ENTRY_STATUS_FAILED as i32 == entry_status::FAILED as i32);
    assert!(ImportConflictPolicy::IMPORT_CONFLICT_POLICY_SKIP as i32 == 1);
    assert!(ImportConflictPolicy::IMPORT_CONFLICT_POLICY_RENAME as i32 == 2);
};

/// Longest source name and file name kept of an entry (for display).
const MAX_SHOWN_CHARS: usize = 255;
/// Highest number tried when looking for a free name (`name_2` … `name_999`).
const MAX_RENAME_NUMBER: u32 = 999;
/// The requester is pushed the progress every this many settled entries.
const PROGRESS_EVERY: i32 = 25;

/// What an import does with an entry whose name is taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Leave the existing emoji alone and report the entry as skipped.
    Skip,
    /// Import the entry under the next free numbered name.
    Rename,
}

impl Policy {
    /// From `tokuly.emoji.v1.ImportConflictPolicy` (unspecified means skip).
    pub fn from_proto(value: i32) -> Self {
        if value == 2 { Self::Rename } else { Self::Skip }
    }

    fn code(self) -> i16 {
        match self {
            Self::Skip => 1,
            Self::Rename => 2,
        }
    }
}

/// What is to happen with one entry of a pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Its picture is processed and becomes an emoji called `name`.
    Accept { name: String },
    /// Left out because of a name clash.
    Skip { name: String, reason: &'static str },
    /// Cannot be imported.
    Fail { name: String, reason: &'static str },
}

/// Whether a pack's file name is just a name: no directories, nothing hidden.
fn is_plain_file_name(file_name: &str) -> bool {
    !file_name.is_empty()
        && file_name.len() <= 255
        && !file_name.starts_with('.')
        && file_name != "meta.json"
        && !file_name.chars().any(|c| matches!(c, '/' | '\\') || c.is_control())
}

/// The first of `base_2`, `base_3`, … that nobody has.
fn free_name(base: &str, taken: &HashSet<String>) -> Option<String> {
    (2..=MAX_RENAME_NUMBER).map(|number| validate::numbered(base, number)).find(|candidate| !taken.contains(candidate))
}

/// Decides what happens with each entry of a pack. `files` are the files in
/// the archive with their sizes, `taken` the names in use — it grows with
/// every accepted entry, so a pack cannot clash with itself unnoticed.
pub fn plan(
    entries: &[PackEntry],
    files: &HashMap<String, u64>,
    taken: &mut HashSet<String>,
    policy: Policy,
    max_bytes: u64,
) -> Vec<Plan> {
    let mut from_this_pack: HashSet<String> = HashSet::new();
    entries
        .iter()
        .map(|entry| {
            let Some(name) = validate::import_name(&entry.source_name) else {
                return Plan::Fail { name: String::new(), reason: "NAME_INVALID" };
            };
            let fail = |reason| Plan::Fail { name: name.clone(), reason };
            if !entry.downloaded {
                return fail("NOT_DOWNLOADED");
            }
            if !is_plain_file_name(&entry.file_name) {
                return fail("FILE_NAME_INVALID");
            }
            match files.get(&entry.file_name) {
                None => return fail("FILE_MISSING"),
                Some(size) if *size > max_bytes => return fail("IMAGE_TOO_LARGE"),
                Some(_) => {}
            }
            let accepted = if !taken.contains(&name) {
                name.clone()
            } else {
                let clash = if from_this_pack.contains(&name) { "DUPLICATE_NAME" } else { "NAME_TAKEN" };
                match policy {
                    Policy::Skip => return Plan::Skip { name, reason: clash },
                    Policy::Rename => match free_name(&name, taken) {
                        Some(free) => free,
                        None => return fail(clash),
                    },
                }
            };
            taken.insert(accepted.clone());
            from_this_pack.insert(name);
            Plan::Accept { name: accepted }
        })
        .collect()
}

/// What a pack says about an emoji, kept with the entry until its picture
/// is ready.
#[derive(Debug, Default, Serialize, Deserialize)]
struct EntryMeta {
    /// Stored name of the category it goes into.
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    license: String,
    #[serde(default)]
    sensitive: bool,
    #[serde(default)]
    local_only: bool,
}

macro_rules! import_columns {
    () => {
        "i.id, i.requested_by, i.asset_id, i.conflict_policy, i.category, i.status, i.error, i.source_host, \
         i.total_count, i.imported_count, i.skipped_count, i.failed_count, i.created_at, i.planned_at, i.completed_at"
    };
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ImportRow {
    id: Uuid,
    requested_by: Option<Uuid>,
    asset_id: Uuid,
    conflict_policy: i16,
    category: Option<String>,
    status: i16,
    error: String,
    source_host: String,
    total_count: i32,
    imported_count: i32,
    skipped_count: i32,
    failed_count: i32,
    created_at: Time,
    planned_at: Option<Time>,
    completed_at: Option<Time>,
}

impl ImportRow {
    fn policy(&self) -> Policy {
        Policy::from_proto(i32::from(self.conflict_policy))
    }

    fn settled_count(&self) -> i32 {
        self.imported_count + self.skipped_count + self.failed_count
    }
}

#[derive(Debug, sqlx::FromRow)]
struct EntryRow {
    position: i32,
    source_name: String,
    file_name: String,
    name: String,
    status: i16,
    reason: String,
    meta: serde_json::Value,
    asset_id: Option<Uuid>,
    emoji_id: Option<Uuid>,
}

const ENTRY_COLUMNS: &str = "position, source_name, file_name, name, status, reason, meta, asset_id, emoji_id";

impl EntryRow {
    fn to_proto(&self) -> ImportEntry {
        ImportEntry {
            source_name: self.source_name.clone(),
            file_name: self.file_name.clone(),
            status: i32::from(self.status).into(),
            name: self.name.clone(),
            reason: self.reason.clone(),
            emoji_id: self.emoji_id.map(|id| id.to_string()).unwrap_or_default(),
            ..Default::default()
        }
    }
}

/// How an entry ended.
enum Outcome {
    Imported { emoji_id: Uuid, name: String, renamed: bool },
    Skipped(&'static str),
    Failed(String),
}

/// Why reading a pack stopped.
enum Halt {
    /// The pack itself is at fault: the import fails, trying again is pointless.
    Pack(PackError),
    /// Something else went wrong: the job is retried.
    Retry(AppError),
}

impl From<AppError> for Halt {
    fn from(error: AppError) -> Self {
        Self::Retry(error)
    }
}

impl From<sqlx::Error> for Halt {
    fn from(error: sqlx::Error) -> Self {
        Self::Retry(error.into())
    }
}

impl From<PackError> for Halt {
    fn from(error: PackError) -> Self {
        Self::Pack(error)
    }
}

/// The media holder under which a running import keeps its archive.
fn archive_holder(import_id: Uuid) -> String {
    format!("emoji:import:{import_id}")
}

/// Runs file work on the blocking pool.
pub(crate) async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> AppResult<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| AppError::internal(anyhow::anyhow!("blocking task failed: {e}")))
}

fn shown(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(MAX_SHOWN_CHARS).collect()
}

async fn lock_import(tx: &mut Tx<'_>, import_id: Uuid) -> AppResult<Option<ImportRow>> {
    Ok(sqlx::query_as(concat!("SELECT ", import_columns!(), " FROM emoji_imports i WHERE i.id = $1 FOR UPDATE"))
        .bind(import_id)
        .fetch_optional(&mut **tx)
        .await?)
}

/// Marks a running import COMPLETED once the pack was read and no entry is
/// waiting any more, and lets go of the archive. Returns whether it did.
async fn complete_if_done(tx: &mut Tx<'_>, import_id: Uuid) -> AppResult<bool> {
    let completed = sqlx::query(
        "UPDATE emoji_imports SET status = 3, completed_at = now()
         WHERE id = $1 AND status = 2 AND planned_at IS NOT NULL
           AND NOT EXISTS (SELECT 1 FROM emoji_import_entries WHERE import_id = $1 AND status = 1)",
    )
    .bind(import_id)
    .execute(&mut **tx)
    .await?
    .rows_affected()
        == 1;
    if completed {
        assets::detach_holder(&mut **tx, &archive_holder(import_id)).await?;
    }
    Ok(completed)
}

impl Emoji {
    async fn import_protos(&self, rows: &[ImportRow]) -> AppResult<Vec<Import>> {
        let requester_ids: Vec<Uuid> = rows.iter().filter_map(|row| row.requested_by).collect();
        let requesters = tokuly_identity::summaries(&self.core, &requester_ids).await?;
        Ok(rows
            .iter()
            .map(|row| Import {
                id: row.id.to_string(),
                status: i32::from(row.status).into(),
                on_conflict: i32::from(row.conflict_policy).into(),
                category: row.category.clone().unwrap_or_default(),
                source_host: row.source_host.clone(),
                total_count: row.total_count,
                imported_count: row.imported_count,
                skipped_count: row.skipped_count,
                failed_count: row.failed_count,
                error: row.error.clone(),
                requested_by: row.requested_by.and_then(|id| requesters.get(&id).cloned()).into(),
                created_at: ts(row.created_at),
                completed_at: opt_ts(row.completed_at),
                ..Default::default()
            })
            .collect())
    }

    async fn import_proto(&self, row: &ImportRow) -> AppResult<Import> {
        Ok(self.import_protos(std::slice::from_ref(row)).await?.pop().unwrap_or_default())
    }

    async fn find_import(&self, import_id: Uuid) -> AppResult<Option<ImportRow>> {
        Ok(sqlx::query_as(concat!("SELECT ", import_columns!(), " FROM emoji_imports i WHERE i.id = $1"))
            .bind(import_id)
            .fetch_optional(&self.core.db)
            .await?)
    }

    /// Registers an import of the archive `asset_id` (an upload of the
    /// moderator) and queues the job that reads it.
    /// Errors: the `ASSET_*` reasons, `ARCHIVE_TOO_LARGE`.
    pub(crate) async fn begin_import(
        &self,
        moderator_id: Uuid,
        asset_id: Uuid,
        policy: Policy,
        category: Option<String>,
    ) -> AppResult<Import> {
        let core = &self.core;
        let upload =
            assets::require_ready(core, asset_id, moderator_id, &[AssetPurpose::ASSET_PURPOSE_CHAT_FILE]).await?;
        ensure!(
            upload.size_bytes <= self.cfg.import_max_bytes(),
            invalid,
            "ARCHIVE_TOO_LARGE",
            "an emoji pack is at most {} MB",
            self.cfg.import_max_mb
        );
        let mut tx = core.db.begin().await?;
        let row: ImportRow = sqlx::query_as(concat!(
            "INSERT INTO emoji_imports AS i (requested_by, asset_id, conflict_policy, category)
             VALUES ($1, $2, $3, $4)
             RETURNING ",
            import_columns!()
        ))
        .bind(moderator_id)
        .bind(asset_id)
        .bind(policy.code())
        .bind(&category)
        .fetch_one(&mut *tx)
        .await?;
        // The archive must outlive the import, whatever its uploader does.
        assets::attach(&mut *tx, asset_id, &archive_holder(row.id)).await?;
        tx.commit().await?;
        if let Err(error) = core.jobs.enqueue(JOB_IMPORT, &ImportJob { import_id: row.id }).await {
            self.fail_import(row.id, "INTERNAL").await?;
            return Err(error);
        }
        self.import_proto(&row).await
    }

    /// An import with one page of its entries, in pack order.
    /// Errors: `IMPORT_NOT_FOUND`.
    pub(crate) async fn import_view(
        &self,
        import_id: Uuid,
        page: &Page,
    ) -> AppResult<(Import, Vec<ImportEntry>, PageInfo)> {
        let row = self.find_import(import_id).await?.or_not_found("IMPORT_NOT_FOUND", "import not found")?;
        let after: Option<i32> = page.decode()?;
        let mut entries: Vec<EntryRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {ENTRY_COLUMNS} FROM emoji_import_entries
             WHERE import_id = $1 AND ($2::int IS NULL OR position > $2)
             ORDER BY position
             LIMIT $3"
        )))
        .bind(import_id)
        .bind(after)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let page = page.finish(&mut entries, |entry| entry.position);
        Ok((self.import_proto(&row).await?, entries.iter().map(EntryRow::to_proto).collect(), page))
    }

    /// Imports, newest first.
    pub(crate) async fn imports_page(&self, page: &Page) -> AppResult<(Vec<Import>, PageInfo)> {
        let before: Option<Uuid> = page.decode()?;
        let mut rows: Vec<ImportRow> = sqlx::query_as(concat!(
            "SELECT ",
            import_columns!(),
            " FROM emoji_imports i WHERE ($1::uuid IS NULL OR i.id < $1) ORDER BY i.id DESC LIMIT $2"
        ))
        .bind(before)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let page = page.finish(&mut rows, |row| row.id);
        Ok((self.import_protos(&rows).await?, page))
    }

    /// Shows the moderator who started an import where it stands. Progress
    /// is pushed every few entries; the end always.
    async fn push_import(&self, import_id: Uuid, finished: bool) {
        let pushed: AppResult<()> = async {
            let Some(row) = self.find_import(import_id).await? else { return Ok(()) };
            let Some(requester) = row.requested_by else { return Ok(()) };
            if finished || row.settled_count() % PROGRESS_EVERY == 0 {
                let event = EmojiEvent { event: Some(self.import_proto(&row).await?.into()), ..Default::default() };
                self.core.rt.notify_user(requester, Body::Emoji(Box::new(event))).await;
            }
            Ok(())
        }
        .await;
        if let Err(e) = pushed {
            tracing::warn!(%import_id, error = %e, "import progress was not pushed");
        }
    }

    /// Ends an import that cannot go on: the pack is unreadable, or its job
    /// never got anywhere.
    async fn fail_import(&self, import_id: Uuid, reason: &str) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let failed = sqlx::query(
            "UPDATE emoji_imports SET status = 4, error = $2, completed_at = now() WHERE id = $1 AND status IN (1, 2)",
        )
        .bind(import_id)
        .bind(reason)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        if failed {
            assets::detach_holder(&mut *tx, &archive_holder(import_id)).await?;
        }
        tx.commit().await?;
        if failed {
            self.push_import(import_id, true).await;
        }
        Ok(())
    }

    /// The `emoji.import` job: reads the pack and hands its pictures to the
    /// media domain. Safe to run again for the same import — it continues
    /// where an earlier run stopped, and does nothing once the import ended.
    pub async fn run_import(&self, import_id: Uuid) -> AppResult<()> {
        let claimed: Option<ImportRow> = sqlx::query_as(concat!(
            "UPDATE emoji_imports i SET status = 2 WHERE i.id = $1 AND i.status IN (1, 2) RETURNING ",
            import_columns!()
        ))
        .bind(import_id)
        .fetch_optional(&self.core.db)
        .await?;
        let Some(import) = claimed else { return Ok(()) };
        match self.unpack(&import).await {
            Ok(()) => {}
            Err(Halt::Pack(error)) => {
                tracing::warn!(%import_id, %error, "emoji pack could not be read");
                return self.fail_import(import_id, error.reason()).await;
            }
            Err(Halt::Retry(error)) => return Err(error),
        }
        // Pictures that were processed before their entry knew its upload
        // (and packs without a single usable entry) are settled here.
        self.settle_import(import_id).await
    }

    async fn unpack(&self, import: &ImportRow) -> Result<(), Halt> {
        let core = &self.core;
        let Some(owner_id) = import.requested_by else {
            return Err(Halt::Pack(PackError::Io("the account that started the import is gone".into())));
        };
        let Some(upload) = assets::find(core, import.asset_id).await? else {
            return Err(Halt::Pack(PackError::ArchiveInvalid));
        };
        let dir =
            tempfile::tempdir().map_err(|e| AppError::internal(anyhow::anyhow!("import scratch directory: {e}")))?;
        let archive_path = dir.path().join("pack.zip");
        core.storage.download(Bucket::Private, &upload.original_key, &archive_path).await?;
        let archive = blocking(move || Archive::open(&archive_path)).await??;
        let archive = Arc::new(Mutex::new(archive));
        if import.planned_at.is_none() {
            self.plan_import(import, &archive).await?;
        }
        self.extract_entries(import, owner_id, &archive, dir.path()).await
    }

    /// Reads `meta.json`, decides what happens with every entry and stores
    /// the result. Categories the pack needs are created here.
    async fn plan_import(&self, import: &ImportRow, archive: &Arc<Mutex<Archive>>) -> Result<(), Halt> {
        let max_entries = self.cfg.import_max_entries;
        let (pack, files) = {
            let archive = archive.clone();
            blocking(move || {
                let mut archive = archive.lock();
                let pack = archive.meta(max_entries)?;
                Ok::<_, PackError>((pack, archive.file_sizes()))
            })
            .await??
        };

        let mut tx = self.core.db.begin().await?;
        let mut fx = Effects::default();
        // A job delivered twice plans once: the second waits here and then
        // sees the first one's plan.
        let Some(current) = lock_import(&mut tx, import.id).await? else { return Ok(()) };
        if current.planned_at.is_some() {
            return Ok(());
        }
        let mut taken: HashSet<String> = sqlx::query_scalar::<_, String>("SELECT name FROM emojis WHERE state <> 3")
            .fetch_all(&mut *tx)
            .await?
            .into_iter()
            .collect();
        let plans = plan(&pack.entries, &files, &mut taken, import.policy(), self.cfg.max_image_bytes() as u64);

        let forced = match &import.category {
            Some(name) => Some(categories::ensure(&mut tx, &mut fx, name).await?),
            None => None,
        };
        let mut pack_categories: HashMap<&str, Option<String>> = HashMap::new();
        let (mut positions, mut source_names, mut file_names, mut names) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let (mut statuses, mut reasons, mut metas) = (Vec::new(), Vec::new(), Vec::new());
        let (mut skipped, mut failed) = (0, 0);
        for (position, (entry, planned)) in pack.entries.iter().zip(&plans).enumerate() {
            let (name, entry_state, reason) = match planned {
                Plan::Accept { name } => (name.as_str(), entry_status::PROCESSING, ""),
                Plan::Skip { name, reason } => {
                    skipped += 1;
                    (name.as_str(), entry_status::SKIPPED, *reason)
                }
                Plan::Fail { name, reason } => {
                    failed += 1;
                    (name.as_str(), entry_status::FAILED, *reason)
                }
            };
            let mut meta = EntryMeta::default();
            if entry_state == entry_status::PROCESSING {
                let category = match (&forced, &entry.category) {
                    (Some(forced), _) => forced.clone(),
                    (None, None) => None,
                    (None, Some(named)) => match pack_categories.get(named.as_str()) {
                        Some(known) => known.clone(),
                        None => {
                            let stored = categories::ensure(&mut tx, &mut fx, named).await?;
                            pack_categories.insert(named.as_str(), stored.clone());
                            stored
                        }
                    },
                };
                meta = EntryMeta {
                    category,
                    aliases: validate::lenient_aliases(&entry.aliases),
                    license: validate::lenient_license(&entry.license),
                    sensitive: entry.sensitive,
                    local_only: entry.local_only,
                };
            }
            positions.push(position as i32);
            source_names.push(shown(&entry.source_name));
            file_names.push(shown(&entry.file_name));
            names.push(name.to_owned());
            statuses.push(entry_state);
            reasons.push(reason.to_owned());
            metas.push(
                serde_json::to_value(&meta).map_err(|e| AppError::internal(anyhow::anyhow!("entry encode: {e}")))?,
            );
        }
        sqlx::query(
            "INSERT INTO emoji_import_entries (import_id, position, source_name, file_name, name, status, reason, meta)
             SELECT $1, listed.*
             FROM unnest($2::int[], $3::text[], $4::text[], $5::text[], $6::smallint[], $7::text[], $8::jsonb[]) AS listed",
        )
        .bind(import.id)
        .bind(&positions)
        .bind(&source_names)
        .bind(&file_names)
        .bind(&names)
        .bind(&statuses)
        .bind(&reasons)
        .bind(&metas)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE emoji_imports
             SET planned_at = now(), source_host = $2, total_count = $3, skipped_count = $4, failed_count = $5
             WHERE id = $1",
        )
        .bind(import.id)
        .bind(shown(&pack.host))
        .bind(pack.entries.len() as i32)
        .bind(skipped)
        .bind(failed)
        .execute(&mut *tx)
        .await?;
        self.commit(tx, fx).await?;
        Ok(())
    }

    /// Unpacks the image of every accepted entry that has no upload yet and
    /// hands it to the media domain.
    async fn extract_entries(
        &self,
        import: &ImportRow,
        owner_id: Uuid,
        archive: &Arc<Mutex<Archive>>,
        scratch: &Path,
    ) -> Result<(), Halt> {
        let core = &self.core;
        let waiting: Vec<(i32, String)> = sqlx::query_as(
            "SELECT position, file_name FROM emoji_import_entries
             WHERE import_id = $1 AND status = 1 AND asset_id IS NULL
             ORDER BY position",
        )
        .bind(import.id)
        .fetch_all(&core.db)
        .await?;
        let max_bytes = self.cfg.max_image_bytes() as u64;
        for (position, file_name) in waiting {
            let image_path = scratch.join(format!("entry-{position}"));
            let unpacked = {
                let (archive, file_name, image_path) = (archive.clone(), file_name.clone(), image_path.clone());
                blocking(move || archive.lock().extract(&file_name, &image_path, max_bytes)).await?
            };
            let failure = match unpacked {
                Err(error) => Some(error.reason().to_owned()),
                Ok(_) => match create_asset_from_file(core, owner_id, PICTURE_PURPOSE, &image_path, &file_name).await {
                    Ok(upload) => {
                        sqlx::query(
                            "UPDATE emoji_import_entries SET asset_id = $3, extracted_at = now()
                             WHERE import_id = $1 AND position = $2 AND status = 1",
                        )
                        .bind(import.id)
                        .bind(position)
                        .bind(upload.id)
                        .execute(&core.db)
                        .await?;
                        None
                    }
                    // What the media domain refuses about the file is the
                    // entry's failure, not the import's.
                    Err(error) if error.code == ErrorCode::InvalidArgument => Some(error.reason.into_owned()),
                    Err(error) => return Err(Halt::Retry(error)),
                },
            };
            // The scratch directory goes with the job; this keeps it small meanwhile.
            let _ = tokio::fs::remove_file(&image_path).await;
            if let Some(reason) = failure {
                self.fail_entry(import.id, position, &reason).await?;
            }
        }
        Ok(())
    }

    /// Settles an entry that never got an upload.
    async fn fail_entry(&self, import_id: Uuid, position: i32, reason: &str) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        if lock_import(&mut tx, import_id).await?.is_none() {
            return Ok(());
        }
        let failed = sqlx::query(
            "UPDATE emoji_import_entries SET status = 5, reason = $3 WHERE import_id = $1 AND position = $2 AND status = 1",
        )
        .bind(import_id)
        .bind(position)
        .bind(reason)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        if failed {
            sqlx::query("UPDATE emoji_imports SET failed_count = failed_count + 1 WHERE id = $1")
                .bind(import_id)
                .execute(&mut *tx)
                .await?;
        }
        let finished = complete_if_done(&mut tx, import_id).await?;
        tx.commit().await?;
        self.push_import(import_id, finished).await;
        Ok(())
    }

    /// `media.asset.ready` / `media.asset.failed`: if the upload belongs to
    /// an entry that is waiting for it, the entry is settled.
    pub(crate) async fn settle_asset(&self, asset_id: Uuid) -> AppResult<()> {
        let waiting: Option<(Uuid, i32)> =
            sqlx::query_as("SELECT import_id, position FROM emoji_import_entries WHERE asset_id = $1 AND status = 1")
                .bind(asset_id)
                .fetch_optional(&self.core.db)
                .await?;
        match waiting {
            Some((import_id, position)) => self.settle_entry(import_id, position).await,
            None => Ok(()),
        }
    }

    /// Settles every entry of an import whose picture is done, and ends the
    /// import if nothing is left to wait for. Public so tests (and tooling)
    /// can move an import along without a running event relay.
    pub async fn settle_import(&self, import_id: Uuid) -> AppResult<()> {
        let waiting: Vec<i32> = sqlx::query_scalar(
            "SELECT position FROM emoji_import_entries
             WHERE import_id = $1 AND status = 1 AND asset_id IS NOT NULL
             ORDER BY position",
        )
        .bind(import_id)
        .fetch_all(&self.core.db)
        .await?;
        for position in waiting {
            self.settle_entry(import_id, position).await?;
        }
        let mut tx = self.core.db.begin().await?;
        let finished = lock_import(&mut tx, import_id).await?.is_some() && complete_if_done(&mut tx, import_id).await?;
        tx.commit().await?;
        if finished {
            self.push_import(import_id, true).await;
        }
        Ok(())
    }

    /// Gives one waiting entry its outcome, if its picture is done: the
    /// emoji is created, or the entry is skipped or failed. Does nothing
    /// while the picture is still being processed, or when the entry was
    /// settled already.
    async fn settle_entry(&self, import_id: Uuid, position: i32) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let mut fx = Effects::default();
        // Entries of one import settle one after the other: the counters and
        // the "is this the last one" check need a stable view.
        let Some(import) = lock_import(&mut tx, import_id).await? else { return Ok(()) };
        let entry: Option<EntryRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {ENTRY_COLUMNS} FROM emoji_import_entries WHERE import_id = $1 AND position = $2 AND status = 1 FOR UPDATE"
        )))
        .bind(import_id)
        .bind(position)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(entry) = entry else { return Ok(()) };
        let Some(asset_id) = entry.asset_id else { return Ok(()) };

        let outcome = match assets::find(&self.core, asset_id).await? {
            None => Outcome::Failed("FILE_UNREADABLE".to_owned()),
            Some(upload) if upload.has_status(AssetStatus::ASSET_STATUS_FAILED) => Outcome::Failed(
                Some(upload.error)
                    .filter(|reason| !reason.is_empty())
                    .unwrap_or_else(|| "PROCESSING_FAILED".to_owned()),
            ),
            Some(upload) if !upload.is_ready() => return Ok(()),
            Some(upload) => match Picture::of(&upload, self.cfg.max_image_bytes()) {
                Ok(picture) => self.admit(&mut tx, &mut fx, &import, &entry, picture).await?,
                // A ZIP inside the pack, or anything else that is no image.
                Err(error) if error.reason == "ASSET_NOT_IMAGE" => Outcome::Failed("UNSUPPORTED_CONTENT".to_owned()),
                Err(error) => Outcome::Failed(error.reason.into_owned()),
            },
        };

        let (entry_state, name, reason, emoji_id, counter) = match &outcome {
            Outcome::Imported { emoji_id, name, renamed } => {
                let entry_state = if *renamed { entry_status::RENAMED } else { entry_status::IMPORTED };
                (entry_state, name.as_str(), "", Some(*emoji_id), "imported_count")
            }
            Outcome::Skipped(reason) => (entry_status::SKIPPED, entry.name.as_str(), *reason, None, "skipped_count"),
            Outcome::Failed(reason) => {
                (entry_status::FAILED, entry.name.as_str(), reason.as_str(), None, "failed_count")
            }
        };
        sqlx::query(
            "UPDATE emoji_import_entries SET status = $3, name = $4, reason = $5, emoji_id = $6
             WHERE import_id = $1 AND position = $2",
        )
        .bind(import_id)
        .bind(position)
        .bind(entry_state)
        .bind(name)
        .bind(reason)
        .bind(emoji_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(sqlx::AssertSqlSafe(format!("UPDATE emoji_imports SET {counter} = {counter} + 1 WHERE id = $1")))
            .bind(import_id)
            .execute(&mut *tx)
            .await?;
        if emoji_id.is_none() {
            // Nothing will ever use the picture.
            fx.discarded_assets.push(asset_id);
        }
        let finished = complete_if_done(&mut tx, import_id).await?;
        self.commit(tx, fx).await?;
        self.push_import(import_id, finished).await;
        Ok(())
    }

    /// Turns a processed picture into an approved emoji. The name planned
    /// for it may have been taken while the picture was processed; the
    /// import's policy applies again then.
    async fn admit(
        &self,
        tx: &mut Tx<'_>,
        fx: &mut Effects,
        import: &ImportRow,
        entry: &EntryRow,
        picture: Picture,
    ) -> AppResult<Outcome> {
        let meta: EntryMeta = serde_json::from_value(entry.meta.clone()).unwrap_or_default();
        // The category may have been deleted since the pack was read.
        let category: Option<String> = match &meta.category {
            Some(name) => {
                sqlx::query_scalar("SELECT name FROM emoji_categories WHERE name = $1 FOR KEY SHARE")
                    .bind(name)
                    .fetch_optional(&mut **tx)
                    .await?
            }
            None => None,
        };
        let own_name = validate::import_name(&entry.source_name).unwrap_or_else(|| entry.name.clone());
        let mut name = entry.name.clone();
        let mut number = 1;
        loop {
            let new = NewEmoji {
                name: name.clone(),
                picture: picture.clone(),
                category: category.clone(),
                aliases: meta.aliases.clone(),
                license: meta.license.clone(),
                sensitive: meta.sensitive,
                local_only: meta.local_only,
                author_id: None,
            };
            let admission = Admission::Catalog(AddedVia::Import);
            if let Some(emoji) = emojis::create(tx, fx, &new, admission, import.requested_by).await? {
                return Ok(Outcome::Imported { emoji_id: emoji.id, renamed: name != own_name, name });
            }
            if import.policy() == Policy::Skip {
                return Ok(Outcome::Skipped("NAME_TAKEN"));
            }
            number += 1;
            if number > MAX_RENAME_NUMBER {
                return Ok(Outcome::Failed("NAME_TAKEN".to_owned()));
            }
            name = validate::numbered(&own_name, number);
        }
    }

    /// The worker's periodic pass over imports: settles entries whose
    /// upload finished without its event being handled, gives up on entries
    /// and imports that have been stuck for hours.
    pub(crate) async fn sweep_imports(&self) -> AppResult<()> {
        let db = &self.core.db;
        let behind: Vec<Uuid> = sqlx::query_scalar(
            "SELECT DISTINCT import_id FROM emoji_import_entries
             WHERE status = 1 AND asset_id IS NOT NULL AND extracted_at < now() - interval '30 seconds'
             LIMIT 50",
        )
        .fetch_all(db)
        .await?;
        for import_id in behind {
            self.settle_import(import_id).await?;
        }

        // Pictures the media domain never finished, and entries whose job
        // died for good before unpacking them.
        let stuck: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM emoji_imports
             WHERE status = 2 AND planned_at IS NOT NULL AND created_at < now() - interval '6 hours'",
        )
        .fetch_all(db)
        .await?;
        for import_id in stuck {
            let mut tx = db.begin().await?;
            let mut fx = Effects::default();
            if lock_import(&mut tx, import_id).await?.is_none() {
                continue;
            }
            let abandoned: Vec<Option<Uuid>> = sqlx::query_scalar(
                "UPDATE emoji_import_entries SET status = 5, reason = 'PROCESSING_TIMEOUT'
                 WHERE import_id = $1 AND status = 1
                 RETURNING asset_id",
            )
            .bind(import_id)
            .fetch_all(&mut *tx)
            .await?;
            sqlx::query("UPDATE emoji_imports SET failed_count = failed_count + $2 WHERE id = $1")
                .bind(import_id)
                .bind(abandoned.len() as i32)
                .execute(&mut *tx)
                .await?;
            fx.discarded_assets.extend(abandoned.into_iter().flatten());
            let finished = complete_if_done(&mut tx, import_id).await?;
            self.commit(tx, fx).await?;
            self.push_import(import_id, finished).await;
        }

        // Imports whose job never read the pack.
        let lost: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM emoji_imports
             WHERE status IN (1, 2) AND planned_at IS NULL AND created_at < now() - interval '6 hours'",
        )
        .fetch_all(db)
        .await?;
        for import_id in lost {
            self.fail_import(import_id, "INTERNAL").await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, file: &str) -> PackEntry {
        PackEntry { source_name: name.into(), file_name: file.into(), downloaded: true, ..Default::default() }
    }

    fn files(names: &[(&str, u64)]) -> HashMap<String, u64> {
        names.iter().map(|(name, size)| (name.to_string(), *size)).collect()
    }

    fn taken(names: &[&str]) -> HashSet<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    fn accept(name: &str) -> Plan {
        Plan::Accept { name: name.into() }
    }

    #[test]
    fn free_names_are_taken_as_they_come() {
        let entries = [entry("BlobCat", "blobcat.png"), entry("party-parrot", "parrot.gif")];
        let files = files(&[("blobcat.png", 10), ("parrot.gif", 10)]);
        let mut names = taken(&["doge"]);
        let plans = plan(&entries, &files, &mut names, Policy::Skip, 100);
        assert_eq!(plans, [accept("blobcat"), accept("party_parrot")]);
        assert!(names.contains("blobcat") && names.contains("party_parrot"));
    }

    #[test]
    fn a_taken_name_is_skipped_or_numbered() {
        let entries = [entry("blobcat", "a.png"), entry("doge", "b.png")];
        let files = files(&[("a.png", 10), ("b.png", 10)]);
        let existing = ["blobcat", "blobcat_2"];

        let skipped = plan(&entries, &files, &mut taken(&existing), Policy::Skip, 100);
        assert_eq!(skipped, [Plan::Skip { name: "blobcat".into(), reason: "NAME_TAKEN" }, accept("doge")]);

        let mut names = taken(&existing);
        let renamed = plan(&entries, &files, &mut names, Policy::Rename, 100);
        assert_eq!(renamed, [accept("blobcat_3"), accept("doge")]);
        assert!(names.contains("blobcat_3"));
    }

    #[test]
    fn a_pack_that_names_an_emoji_twice_clashes_with_itself() {
        // "Cat" and "cat" are the same name here.
        let entries = [entry("cat", "a.png"), entry("Cat", "b.png"), entry("cat", "c.png")];
        let files = files(&[("a.png", 1), ("b.png", 1), ("c.png", 1)]);
        let skipped = plan(&entries, &files, &mut taken(&[]), Policy::Skip, 100);
        assert_eq!(skipped[0], accept("cat"));
        assert_eq!(skipped[1], Plan::Skip { name: "cat".into(), reason: "DUPLICATE_NAME" });
        let renamed = plan(&entries, &files, &mut taken(&[]), Policy::Rename, 100);
        assert_eq!(renamed, [accept("cat"), accept("cat_2"), accept("cat_3")]);
    }

    #[test]
    fn entries_that_cannot_work_fail_with_their_reason() {
        let mut gone = entry("gone", "gone.png");
        gone.downloaded = false;
        let entries = [
            entry("ねこ", "neko.png"),
            gone,
            entry("escape", "../escape.png"),
            entry("hidden", ".hidden.png"),
            entry("meta", "meta.json"),
            entry("missing", "missing.png"),
            entry("huge", "huge.png"),
            entry("fine", "fine.png"),
        ];
        let files =
            files(&[("huge.png", 101), ("fine.png", 100), ("meta.json", 5), ("../escape.png", 5), (".hidden.png", 5)]);
        let plans = plan(&entries, &files, &mut taken(&[]), Policy::Rename, 100);
        let reasons: Vec<&str> = plans
            .iter()
            .map(|plan| match plan {
                Plan::Fail { reason, .. } => *reason,
                Plan::Accept { .. } => "accepted",
                Plan::Skip { .. } => "skipped",
            })
            .collect();
        assert_eq!(
            reasons,
            [
                "NAME_INVALID",
                "NOT_DOWNLOADED",
                "FILE_NAME_INVALID",
                "FILE_NAME_INVALID",
                "FILE_NAME_INVALID",
                "FILE_MISSING",
                "IMAGE_TOO_LARGE",
                "accepted"
            ]
        );
        // A name nobody could use is reported as empty.
        assert_eq!(plans[0], Plan::Fail { name: String::new(), reason: "NAME_INVALID" });
    }

    #[test]
    fn a_failed_entry_does_not_hold_its_name() {
        let entries = [entry("cat", "missing.png"), entry("cat", "cat.png")];
        let plans = plan(&entries, &files(&[("cat.png", 1)]), &mut taken(&[]), Policy::Skip, 100);
        assert_eq!(plans[1], accept("cat"));
    }

    #[test]
    fn policies_come_from_the_api_enum() {
        assert_eq!(Policy::from_proto(0), Policy::Skip);
        assert_eq!(Policy::from_proto(1), Policy::Skip);
        assert_eq!(Policy::from_proto(2), Policy::Rename);
        assert_eq!(Policy::from_proto(Policy::Rename.code().into()), Policy::Rename);
    }

    #[test]
    fn shown_text_is_bounded_and_printable() {
        assert_eq!(shown("a\u{0}b\nc"), "abc");
        assert_eq!(shown(&"x".repeat(1000)).len(), MAX_SHOWN_CHARS);
    }
}
