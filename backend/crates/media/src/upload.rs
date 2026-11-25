//! The upload lifecycle: create → (parts) → complete, or abort.
//!
//! Bytes never pass through this service. `create` hands the client
//! presigned URLs for the private bucket; `complete` then inspects what was
//! actually stored — size, checksum, real content type — before the asset
//! is allowed into processing.
use std::time::Duration;

use tokuly_core::{prelude::*, storage::Bucket};
use tokuly_proto::media::{AssetKind, AssetPurpose, AssetStatus, CreateUploadResponse, GetUploadPartsResponse, UploadPart};

use crate::{
    Media,
    assets::{self, ASSET_COLUMNS, AssetRow},
    lifecycle, recipe, store,
};

const UPLOAD_URL_TTL: Duration = Duration::from_secs(lifecycle::UPLOAD_URL_SECONDS);
/// Unfinished uploads one user may have at a time.
const MAX_OPEN_UPLOADS: i64 = 100;
/// Uploads one user may start per window.
const UPLOADS_PER_WINDOW: u32 = 300;
const UPLOAD_WINDOW: Duration = Duration::from_secs(600);
/// Attempts to complete one upload per minute while it is still unfinished.
/// Each attempt reads (and copies) the stored bytes; a client completes once
/// and retries a few times at most.
const COMPLETIONS_PER_MINUTE: u32 = 10;
/// Part URLs signed per `GetUploadParts` call.
pub const MAX_PARTS_PER_CALL: usize = 100;
/// Checksums of uploads up to this size are verified while completing the
/// upload; larger ones by the worker, which has to read the file anyway.
const INLINE_CHECKSUM_MAX_BYTES: i64 = 64 * 1024 * 1024;

/// Whether the client's checksum still has to be verified by the worker.
pub(crate) fn checksum_deferred(row: &AssetRow) -> bool {
    !row.sha256.is_empty() && row.size_bytes > INLINE_CHECKSUM_MAX_BYTES
}

/// What a client declares when starting an upload.
pub(crate) struct NewUpload<'a> {
    pub purpose: Option<AssetPurpose>,
    pub file_name: &'a str,
    pub content_type: &'a str,
    pub size_bytes: i64,
    pub sha256: &'a str,
}

/// A validated upload, ready to be recorded.
struct NewRow<'a> {
    asset_id: Uuid,
    owner_id: Uuid,
    purpose: AssetPurpose,
    kind: AssetKind,
    upload: &'a NewUpload<'a>,
    content_type: &'a str,
    file_name: &'a str,
    key: &'a str,
    multipart_id: Option<&'a str>,
    part_size: i64,
    part_count: i32,
    attrs: serde_json::Value,
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// S3 reports ETags quoted; clients may or may not keep the quotes.
fn normalize_etag(etag: &str) -> &str {
    etag.trim().trim_matches('"')
}

/// Exact length of part `number` (1-based): every part is full-sized except the last.
fn part_length(row: &AssetRow, number: i32) -> i64 {
    if number < row.part_count { row.part_size } else { row.size_bytes - row.part_size * i64::from(row.part_count - 1) }
}

impl Media {
    pub(crate) async fn create_upload(&self, owner_id: Uuid, upload: NewUpload<'_>) -> AppResult<CreateUploadResponse> {
        let core = &self.core;
        let purpose = upload
            .purpose
            .filter(|p| *p != AssetPurpose::ASSET_PURPOSE_UNSPECIFIED)
            .ok_or_else(|| AppError::invalid("PURPOSE_INVALID", "unknown upload purpose"))?;
        let file_name = recipe::sanitize_file_name(upload.file_name);
        let declared = recipe::classify_declared(upload.content_type, &file_name);
        let Some(max_bytes) = recipe::max_bytes(&self.live(), purpose, declared.class) else {
            bail!(denied, "PURPOSE_NOT_ALLOWED", "clients cannot upload for this purpose");
        };
        let Some(kind) = recipe::resolve(purpose, declared.class) else {
            bail!(invalid, "UNSUPPORTED_CONTENT_TYPE", "{} cannot be uploaded for this purpose", declared.content_type);
        };
        ensure!(upload.size_bytes > 0, invalid, "FILE_EMPTY", "the file is empty");
        ensure!(upload.size_bytes <= max_bytes, invalid, "FILE_TOO_LARGE", "the file exceeds the limit of {max_bytes} bytes");
        ensure!(upload.sha256.is_empty() || is_sha256_hex(upload.sha256), invalid, "SHA256_INVALID", "sha256 must be 64 hex characters");

        core.cache.rate_limit(&format!("media:upload:{owner_id}"), UPLOADS_PER_WINDOW, UPLOAD_WINDOW).await?;

        let asset_id = new_id();
        let key = store::original_key(asset_id, &recipe::extension_of(&file_name));
        let multipart = upload.size_bytes > self.cfg.multipart_threshold_bytes();
        let (multipart_id, part_size, part_count, put_url) = if multipart {
            let (part_size, part_count) = self.cfg.part_layout(upload.size_bytes);
            let multipart_id = core.storage.create_multipart(Bucket::Private, &key, &declared.content_type).await?;
            (Some(multipart_id), part_size, part_count, String::new())
        } else {
            let url = store::presigner(core).presign_put(&key, &declared.content_type, upload.size_bytes, UPLOAD_URL_TTL).await?;
            (None, 0, 0, url)
        };

        let max_duration_seconds = recipe::max_duration_of(&self.live(), purpose, kind);
        let row = NewRow {
            attrs: recipe::new_asset_attrs(max_duration_seconds, false),
            asset_id,
            owner_id,
            purpose,
            kind,
            upload: &upload,
            content_type: &declared.content_type,
            file_name: &file_name,
            key: &key,
            multipart_id: multipart_id.as_deref(),
            part_size,
            part_count,
        };
        let created_at = match self.insert_upload(row).await {
            Ok(created_at) => created_at,
            Err(error) => {
                // Without a row nobody would ever abort the multipart upload.
                if let Some(multipart_id) = &multipart_id {
                    let _ = core.storage.abort_multipart(Bucket::Private, &key, multipart_id).await;
                }
                return Err(error);
            }
        };

        // A single PUT must repeat the content type the URL was signed for.
        let put_headers = if multipart { Default::default() } else { [("Content-Type".to_owned(), declared.content_type)].into_iter().collect() };
        Ok(CreateUploadResponse {
            asset_id: asset_id.to_string(),
            upload_id: asset_id.to_string(),
            put_url,
            put_headers,
            part_size_bytes: part_size,
            part_count,
            expires_at: ts(created_at + self.cfg.upload_ttl()),
            max_duration_seconds: max_duration_seconds.unwrap_or(0) as i32,
            ..Default::default()
        })
    }

    /// Records a new upload, unless the owner is over their limits: at most
    /// [`MAX_OPEN_UPLOADS`] unfinished uploads, at most
    /// `TOKULY_MEDIA_MAX_UNUSED_GB` in uploads nothing uses yet (unfinished,
    /// processing, or finished and not attached anywhere), and at most
    /// `TOKULY_MEDIA_DAILY_UPLOAD_GB` uploaded in the last 24 hours, used
    /// or not (uploads aborted before completion do not count). The owner's
    /// uploads are counted and the row is inserted under a per-owner lock,
    /// so requests at the same moment cannot pass the check together.
    async fn insert_upload(&self, row: NewRow<'_>) -> AppResult<chrono::DateTime<chrono::Utc>> {
        let mut tx = self.core.db.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('media:uploads:' || $1::text, 0))")
            .bind(row.owner_id)
            .execute(&mut *tx)
            .await?;
        let (open, unused_bytes): (i64, i64) = sqlx::query_as(
            "SELECT count(*) FILTER (WHERE status = $2), COALESCE(sum(size_bytes) FILTER (WHERE holder_count = 0), 0)::bigint
             FROM media_assets
             WHERE owner_id = $1 AND deleted_at IS NULL AND status IN ($2, $3, $4) AND (holder_count = 0 OR status = $2)",
        )
        .bind(row.owner_id)
        .bind(AssetStatus::ASSET_STATUS_UPLOADING as i32)
        .bind(AssetStatus::ASSET_STATUS_PROCESSING as i32)
        .bind(AssetStatus::ASSET_STATUS_READY as i32)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(open < MAX_OPEN_UPLOADS, rate_limited, "TOO_MANY_UPLOADS", "finish or abort your unfinished uploads first");
        ensure!(
            unused_bytes.saturating_add(row.upload.size_bytes) <= self.live().max_unused_bytes(),
            rate_limited,
            "UNUSED_QUOTA_EXCEEDED",
            "use or delete some of your uploads first"
        );
        let uploaded_today: i64 = sqlx::query_scalar(
            "SELECT COALESCE(sum(size_bytes), 0)::bigint FROM media_assets
             WHERE owner_id = $1 AND created_at > now() - interval '24 hours'
               AND NOT (deleted_at IS NOT NULL AND status = $2)",
        )
        .bind(row.owner_id)
        .bind(AssetStatus::ASSET_STATUS_UPLOADING as i32)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(
            uploaded_today.saturating_add(row.upload.size_bytes) <= self.live().daily_upload_bytes(),
            rate_limited,
            "DAILY_UPLOAD_LIMIT_EXCEEDED",
            "you have uploaded as much as one account may in a day; try again later"
        );
        let created_at = sqlx::query_scalar(
            "INSERT INTO media_assets
                 (id, owner_id, purpose, kind, status, content_type, size_bytes, file_name, sha256, original_key, multipart_id, part_size, part_count, attrs)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
             RETURNING created_at",
        )
        .bind(row.asset_id)
        .bind(row.owner_id)
        .bind(row.purpose as i32)
        .bind(row.kind as i32)
        .bind(AssetStatus::ASSET_STATUS_UPLOADING as i32)
        .bind(row.content_type)
        .bind(row.upload.size_bytes)
        .bind(row.file_name)
        .bind(row.upload.sha256.to_ascii_lowercase())
        .bind(row.key)
        .bind(row.multipart_id)
        .bind(row.part_size)
        .bind(row.part_count)
        .bind(&row.attrs)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(created_at)
    }

    /// The caller's upload, whatever state it is in.
    async fn owned_upload(&self, owner_id: Uuid, upload_id: Uuid) -> AppResult<AssetRow> {
        assets::load(&self.core, upload_id)
            .await?
            .filter(|row| row.owner_id == owner_id)
            .or_not_found("UPLOAD_NOT_FOUND", "upload not found")
    }

    pub(crate) async fn upload_parts(&self, owner_id: Uuid, upload_id: Uuid, part_numbers: &[i32]) -> AppResult<GetUploadPartsResponse> {
        let core = &self.core;
        let row = self.owned_upload(owner_id, upload_id).await?;
        ensure!(row.has_status(AssetStatus::ASSET_STATUS_UPLOADING), precondition, "UPLOAD_NOT_ACTIVE", "the upload is already complete");
        let Some(multipart_id) = &row.multipart_id else {
            bail!(precondition, "UPLOAD_NOT_MULTIPART", "this upload uses a single PUT");
        };
        ensure!(part_numbers.len() <= MAX_PARTS_PER_CALL, invalid, "TOO_MANY_PARTS", "at most {MAX_PARTS_PER_CALL} parts per call");
        ensure!(
            part_numbers.iter().all(|n| (1..=row.part_count).contains(n)),
            invalid,
            "PART_NUMBER_INVALID",
            "part numbers run from 1 to {}",
            row.part_count
        );

        let mut completed: Vec<i32> =
            core.storage.list_parts(Bucket::Private, &row.original_key, multipart_id).await?.into_iter().map(|(number, _)| number).collect();
        completed.sort_unstable();
        completed.dedup();

        let mut wanted = part_numbers.to_vec();
        wanted.sort_unstable();
        wanted.dedup();
        let presigner = store::presigner(core);
        let mut parts = Vec::with_capacity(wanted.len());
        for number in wanted {
            let put_url = presigner.presign_part(&row.original_key, multipart_id, number, part_length(&row, number), UPLOAD_URL_TTL).await?;
            parts.push(UploadPart { part_number: number, put_url, ..Default::default() });
        }
        Ok(GetUploadPartsResponse { parts, completed_part_numbers: completed, ..Default::default() })
    }

    /// Finalizes an upload. Idempotent: once the upload is complete, calling
    /// again returns the asset as it is now.
    pub(crate) async fn complete_upload(&self, owner_id: Uuid, upload_id: Uuid, client_parts: &[(i32, String)]) -> AppResult<AssetRow> {
        let core = &self.core;
        let row = self.owned_upload(owner_id, upload_id).await?;
        if !row.has_status(AssetStatus::ASSET_STATUS_UPLOADING) {
            return self.completed_asset(row).await;
        }
        core.cache.rate_limit(&format!("media:complete:{upload_id}"), COMPLETIONS_PER_MINUTE, Duration::from_secs(60)).await?;

        if let Some(multipart_id) = &row.multipart_id {
            self.assemble_parts(&row, multipart_id, client_parts).await?;
        }
        if core.storage.head(Bucket::Private, &row.original_key).await?.is_none() {
            bail!(precondition, "UPLOAD_INCOMPLETE", "the file has not been uploaded yet");
        }
        // The single-PUT URL stays valid after completion; a second PUT
        // through it would replace the bytes checked here (and served to
        // whoever receives the file). Check and keep a copy it cannot reach.
        // A completed multipart upload accepts no more parts.
        let stored_key = match row.multipart_id {
            Some(_) => None,
            None => {
                let key = store::stored_key(row.id, new_id(), &recipe::extension_of(&row.file_name));
                // Fresh metadata: nothing the uploader sent with the PUT
                // (Content-Encoding, Content-Language, Cache-Control…) is
                // kept. Downloads state the detected type themselves.
                core.storage.copy((Bucket::Private, &row.original_key), (Bucket::Private, &key), Some("application/octet-stream")).await?;
                Some(key)
            }
        };
        let key = stored_key.as_deref().unwrap_or(&row.original_key);
        let Some(stored_bytes) = core.storage.head(Bucket::Private, key).await? else {
            bail!(precondition, "UPLOAD_INCOMPLETE", "the file has not been uploaded yet");
        };
        if stored_bytes != row.size_bytes {
            return self.reject_upload(&row, "SIZE_MISMATCH", "the uploaded file does not have the announced size").await;
        }

        let originals = &self.originals;
        let sniffed = recipe::sniff(&originals.read_prefix(key, recipe::SNIFF_BYTES).await?);
        let Some(kind) = recipe::resolve(row.asset_purpose(), sniffed.class) else {
            return self.reject_upload(&row, "UNSUPPORTED_CONTENT", "the uploaded file is not of a type this purpose accepts").await;
        };
        let verify_now = !row.sha256.is_empty() && !checksum_deferred(&row);
        if verify_now && originals.sha256(key).await? != row.sha256 {
            return self.reject_upload(&row, "CHECKSUM_MISMATCH", "the uploaded file does not match its SHA-256").await;
        }

        let upload_key = row.original_key.clone();
        let Some(row) = lifecycle::mark_uploaded(core, row.id, kind, &sniffed.content_type, stored_key.as_deref()).await? else {
            // A concurrent call completed (or deleted) the upload first; it
            // keeps its own copy, and a purge may have run before ours was made.
            if let Some(key) = &stored_key
                && let Err(e) = core.storage.delete(Bucket::Private, key).await
            {
                tracing::warn!(asset_id = %upload_id, error = %e, "could not remove the copy of a lost completion");
            }
            let row = self.owned_upload(owner_id, upload_id).await?;
            return self.completed_asset(row).await;
        };
        if stored_key.is_some()
            && let Err(e) = core.storage.delete(Bucket::Private, &upload_key).await
        {
            // The janitor removes it with whatever a late PUT stores there.
            tracing::debug!(asset_id = %row.id, error = %e, "upload object left for the janitor");
        }
        // Files need no rendering, so they become READY right here. The
        // exception is a large file whose checksum is still to be verified.
        let quick = kind == AssetKind::ASSET_KIND_FILE && !checksum_deferred(&row);
        if quick {
            if let Err(e) = self.run_job(lifecycle::ProcessJob { asset_id: row.id }).await {
                tracing::warn!(asset_id = %row.id, error = %e, "inline processing failed; queued instead");
                lifecycle::enqueue(core, &row).await?;
            }
        } else {
            lifecycle::enqueue(core, &row).await?;
        }
        let asset = assets::find(core, row.id).await?.or_not_found("UPLOAD_NOT_FOUND", "upload not found")?;
        // A file rejected on the spot is reported like the checks above.
        if asset.has_status(AssetStatus::ASSET_STATUS_FAILED) {
            return Err(AppError::invalid(asset.error, "the uploaded file was rejected"));
        }
        Ok(asset)
    }

    /// The idempotent answer for an upload that is already complete. Also
    /// revives processing that lost its job (e.g. the queue was unreachable
    /// right after the upload completed).
    async fn completed_asset(&self, mut row: AssetRow) -> AppResult<AssetRow> {
        if row.has_status(AssetStatus::ASSET_STATUS_PROCESSING) && lifecycle::is_stalled(&self.core, row.id).await? {
            lifecycle::enqueue(&self.core, &row).await?;
        }
        row.sign_private_url(&self.core).await?;
        Ok(row)
    }

    /// Marks the upload FAILED (discarding its bytes) and reports why.
    async fn reject_upload(&self, row: &AssetRow, reason: &'static str, message: &'static str) -> AppResult<AssetRow> {
        lifecycle::fail(&self.core, row.id, reason).await?;
        Err(AppError::invalid(reason, message))
    }

    /// Turns the uploaded parts into the final object. The store is the
    /// source of truth for which parts exist, so a client that lost its
    /// ETags (a resumed upload) can still complete.
    async fn assemble_parts(&self, row: &AssetRow, multipart_id: &str, client_parts: &[(i32, String)]) -> AppResult<()> {
        let storage = &self.core.storage;
        let stored = match storage.list_parts(Bucket::Private, &row.original_key, multipart_id).await {
            Ok(parts) => parts,
            // The multipart upload is gone when an earlier attempt already
            // assembled it but failed before recording that.
            Err(_) if storage.head(Bucket::Private, &row.original_key).await?.is_some() => return Ok(()),
            Err(e) => return Err(e),
        };
        for number in 1..=row.part_count {
            ensure!(stored.iter().any(|(n, _)| *n == number), precondition, "UPLOAD_INCOMPLETE", "part {number} has not been uploaded");
        }
        for (number, etag) in client_parts {
            let matches = stored.iter().any(|(n, stored_etag)| n == number && normalize_etag(stored_etag) == normalize_etag(etag));
            ensure!(matches, invalid, "PART_ETAG_MISMATCH", "part {number} is not the one that was uploaded");
        }
        let parts: Vec<(i32, String)> = stored.into_iter().filter(|(n, _)| (1..=row.part_count).contains(n)).collect();
        match storage.complete_multipart(Bucket::Private, &row.original_key, multipart_id, parts).await {
            Ok(()) => Ok(()),
            Err(_) if storage.head(Bucket::Private, &row.original_key).await?.is_some() => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Cancels an unfinished upload. Idempotent.
    pub(crate) async fn abort_upload(&self, owner_id: Uuid, upload_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        match assets::load(core, upload_id).await?.filter(|row| row.owner_id == owner_id) {
            Some(row) => {
                ensure!(
                    row.has_status(AssetStatus::ASSET_STATUS_UPLOADING),
                    precondition,
                    "UPLOAD_NOT_ACTIVE",
                    "the upload is already complete; delete the asset instead"
                );
                lifecycle::delete(core, row.id, Some(owner_id)).await?;
                Ok(())
            }
            None => {
                let already_aborted: bool =
                    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM media_assets WHERE id = $1 AND owner_id = $2 AND deleted_at IS NOT NULL)")
                        .bind(upload_id)
                        .bind(owner_id)
                        .fetch_one(&core.db)
                        .await?;
                ensure!(already_aborted, not_found, "UPLOAD_NOT_FOUND", "upload not found");
                Ok(())
            }
        }
    }

    /// Unfinished uploads older than the configured lifetime, oldest first.
    pub(crate) async fn stale_uploads(&self, limit: i64) -> AppResult<Vec<AssetRow>> {
        let rows = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {ASSET_COLUMNS} FROM media_assets
             WHERE status = $1 AND deleted_at IS NULL AND created_at < now() - make_interval(secs => $2)
             ORDER BY created_at LIMIT $3"
        )))
        .bind(AssetStatus::ASSET_STATUS_UPLOADING as i32)
        .bind(self.cfg.upload_ttl().as_secs_f64())
        .bind(limit)
        .fetch_all(&self.core.db)
        .await?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums_and_etags_are_validated_loosely_but_safely() {
        assert!(is_sha256_hex(&"aB3f".repeat(16)));
        assert!(!is_sha256_hex("abc"));
        assert!(!is_sha256_hex(&"g".repeat(64)));
        assert_eq!(normalize_etag(" \"abc-2\" "), "abc-2");
        assert_eq!(normalize_etag("abc"), "abc");
    }
}
