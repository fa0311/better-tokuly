//! Exporting the catalogue as a pack (see [`crate::pack`] for the format).
//!
//! `StartExport` records the request and queues the `emoji.export` job. The
//! job writes every approved emoji in scope — metadata into `meta.json`,
//! the picture as the media domain rendered it (WebP, without the
//! metadata an upload may carry) — into a ZIP in the private bucket.
//! `GetExport` hands out a link to it that is valid for an hour; the file
//! itself is deleted after `TOKULY_EMOJI_EXPORT_TTL_HOURS`.
use std::time::Duration;

use tokuly_core::{prelude::*, storage::Bucket};
use tokuly_proto::{
    common::PageInfo,
    emoji::{EmojiEvent, Export},
    realtime::server_frame::Body,
};

use crate::{
    Emoji,
    events::{ExportJob, JOB_EXPORT},
    import::blocking,
    model::{EmojiRow, emoji_columns},
    pack::{ExportedEmoji, PackError, Writer},
};

/// `emoji_exports.status` (`tokuly.emoji.v1.ExportStatus`).
mod status {
    pub const QUEUED: i16 = 1;
    pub const RUNNING: i16 = 2;
    pub const READY: i16 = 3;
    pub const FAILED: i16 = 4;
    pub const EXPIRED: i16 = 5;
}

const _: () = {
    use tokuly_proto::emoji::ExportStatus;
    // SQL in this file compares the column with literals.
    assert!(ExportStatus::EXPORT_STATUS_QUEUED as i32 == status::QUEUED as i32);
    assert!(ExportStatus::EXPORT_STATUS_RUNNING as i32 == status::RUNNING as i32);
    assert!(ExportStatus::EXPORT_STATUS_READY as i32 == status::READY as i32);
    assert!(ExportStatus::EXPORT_STATUS_FAILED as i32 == status::FAILED as i32);
    assert!(ExportStatus::EXPORT_STATUS_EXPIRED as i32 == status::EXPIRED as i32);
};

/// How long a download link handed out by `GetExport` works.
const DOWNLOAD_URL_TTL: Duration = Duration::from_secs(3600);
/// Every rendition the media domain publishes of a picture is WebP.
const RENDITION_CONTENT_TYPE: &str = "image/webp";

/// Which emoji an export covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Everything,
    /// The emoji without a category.
    Uncategorized,
    /// One category, by its stored name.
    Category(String),
}

impl Scope {
    /// The `emoji_exports.category` column: NULL, `''` or the name.
    fn column(&self) -> Option<&str> {
        match self {
            Self::Everything => None,
            Self::Uncategorized => Some(""),
            Self::Category(name) => Some(name),
        }
    }
}

macro_rules! export_columns {
    () => {
        "x.id, x.requested_by, x.category, x.status, x.object_key, x.size_bytes, x.emoji_count, x.error, \
         x.created_at, x.completed_at, x.expires_at"
    };
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ExportRow {
    id: Uuid,
    requested_by: Option<Uuid>,
    category: Option<String>,
    status: i16,
    object_key: String,
    size_bytes: i64,
    emoji_count: i32,
    error: String,
    created_at: Time,
    completed_at: Option<Time>,
    expires_at: Option<Time>,
}

/// The finished archive.
struct Written {
    object_key: String,
    size_bytes: i64,
    emoji_count: i32,
}

fn pack_error(error: PackError) -> AppError {
    AppError::internal(anyhow::anyhow!("writing the emoji export failed: {error}"))
}

/// The media bucket key of a published rendition, from its URL. `None` for
/// a URL that does not point into the media bucket.
fn rendition_key(core: &Core, url: &str) -> Option<String> {
    let key = url.strip_prefix(core.storage.media_url("").as_str())?;
    (!key.is_empty()).then(|| key.to_owned())
}

/// Where an export's archive lives in the private bucket.
fn object_key(export_id: Uuid) -> String {
    format!("emoji/exports/{export_id}.zip")
}

impl Emoji {
    async fn export_protos(&self, rows: &[ExportRow]) -> AppResult<Vec<Export>> {
        let requester_ids: Vec<Uuid> = rows.iter().filter_map(|row| row.requested_by).collect();
        let requesters = tokuly_identity::summaries(&self.core, &requester_ids).await?;
        let mut exports = Vec::with_capacity(rows.len());
        for row in rows {
            let downloadable = row.status == status::READY && row.expires_at.is_none_or(|at| at > now());
            let download_url = if downloadable {
                self.core.storage.presign_get(Bucket::Private, &row.object_key, DOWNLOAD_URL_TTL).await?
            } else {
                String::new()
            };
            exports.push(Export {
                id: row.id.to_string(),
                status: i32::from(row.status).into(),
                by_category: row.category.is_some(),
                category: row.category.clone().unwrap_or_default(),
                emoji_count: row.emoji_count,
                size_bytes: row.size_bytes,
                download_url,
                error: row.error.clone(),
                requested_by: row.requested_by.and_then(|id| requesters.get(&id).cloned()).into(),
                created_at: ts(row.created_at),
                completed_at: opt_ts(row.completed_at),
                expires_at: opt_ts(row.expires_at),
                ..Default::default()
            });
        }
        Ok(exports)
    }

    async fn find_export(&self, export_id: Uuid) -> AppResult<Option<ExportRow>> {
        Ok(sqlx::query_as(concat!("SELECT ", export_columns!(), " FROM emoji_exports x WHERE x.id = $1"))
            .bind(export_id)
            .fetch_optional(&self.core.db)
            .await?)
    }

    /// Records an export request and queues the job that writes the archive.
    pub(crate) async fn begin_export(&self, moderator_id: Uuid, scope: Scope) -> AppResult<Export> {
        let row: ExportRow = sqlx::query_as(concat!(
            "INSERT INTO emoji_exports AS x (requested_by, category) VALUES ($1, $2) RETURNING ",
            export_columns!()
        ))
        .bind(moderator_id)
        .bind(scope.column())
        .fetch_one(&self.core.db)
        .await?;
        if let Err(error) = self.core.jobs.enqueue(JOB_EXPORT, &ExportJob { export_id: row.id }).await {
            self.fail_export(row.id).await?;
            return Err(error);
        }
        Ok(self.export_protos(std::slice::from_ref(&row)).await?.pop().unwrap_or_default())
    }

    /// One export. Errors: `EXPORT_NOT_FOUND`.
    pub(crate) async fn export_view(&self, export_id: Uuid) -> AppResult<Export> {
        let row = self.find_export(export_id).await?.or_not_found("EXPORT_NOT_FOUND", "export not found")?;
        Ok(self.export_protos(std::slice::from_ref(&row)).await?.pop().unwrap_or_default())
    }

    /// Exports, newest first.
    pub(crate) async fn exports_page(&self, page: &Page) -> AppResult<(Vec<Export>, PageInfo)> {
        let before: Option<Uuid> = page.decode()?;
        let mut rows: Vec<ExportRow> = sqlx::query_as(concat!(
            "SELECT ",
            export_columns!(),
            " FROM emoji_exports x WHERE ($1::uuid IS NULL OR x.id < $1) ORDER BY x.id DESC LIMIT $2"
        ))
        .bind(before)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let page = page.finish(&mut rows, |row| row.id);
        Ok((self.export_protos(&rows).await?, page))
    }

    async fn fail_export(&self, export_id: Uuid) -> AppResult<()> {
        sqlx::query("UPDATE emoji_exports SET status = 4, error = 'INTERNAL', completed_at = now() WHERE id = $1 AND status IN (1, 2)")
            .bind(export_id)
            .execute(&self.core.db)
            .await?;
        Ok(())
    }

    /// Shows the moderator who asked for an export how it ended.
    async fn push_export(&self, export_id: Uuid) {
        let pushed: AppResult<()> = async {
            let Some(row) = self.find_export(export_id).await? else { return Ok(()) };
            let Some(requester) = row.requested_by else { return Ok(()) };
            let export = self.export_protos(std::slice::from_ref(&row)).await?.pop().unwrap_or_default();
            let event = EmojiEvent { event: Some(export.into()), ..Default::default() };
            self.core.rt.notify_user(requester, Body::Emoji(Box::new(event))).await;
            Ok(())
        }
        .await;
        if let Err(e) = pushed {
            tracing::warn!(%export_id, error = %e, "export result was not pushed");
        }
    }

    /// The `emoji.export` job. An export that cannot be written is marked
    /// FAILED rather than retried: the moderator sees it and starts another.
    pub async fn run_export(&self, export_id: Uuid) -> AppResult<()> {
        let claimed: Option<ExportRow> = sqlx::query_as(concat!(
            "UPDATE emoji_exports x SET status = 2, started_at = now() WHERE x.id = $1 AND x.status IN (1, 2) RETURNING ",
            export_columns!()
        ))
        .bind(export_id)
        .fetch_optional(&self.core.db)
        .await?;
        let Some(export) = claimed else { return Ok(()) };
        match self.write_export(&export).await {
            Ok(written) => {
                sqlx::query(
                    "UPDATE emoji_exports
                     SET status = 3, object_key = $2, size_bytes = $3, emoji_count = $4, completed_at = now(),
                         expires_at = now() + make_interval(hours => $5::int)
                     WHERE id = $1",
                )
                .bind(export_id)
                .bind(&written.object_key)
                .bind(written.size_bytes)
                .bind(written.emoji_count)
                .bind(self.cfg.export_ttl_hours)
                .execute(&self.core.db)
                .await?;
            }
            Err(error) => {
                tracing::error!(%export_id, %error, "emoji export failed");
                self.fail_export(export_id).await?;
            }
        }
        self.push_export(export_id).await;
        Ok(())
    }

    /// Writes the archive to a scratch file and stores it in the private bucket.
    async fn write_export(&self, export: &ExportRow) -> AppResult<Written> {
        let core = &self.core;
        let emojis: Vec<EmojiRow> = sqlx::query_as(concat!(
            "SELECT ",
            emoji_columns!(),
            " FROM emojis e
              WHERE e.state = 2
                AND ($1::text IS NULL OR CASE WHEN $1 = '' THEN e.category IS NULL ELSE e.category = $1 END)
              ORDER BY e.name"
        ))
        .bind(&export.category)
        .fetch_all(&core.db)
        .await?;

        let dir = tempfile::tempdir()?;
        let path = dir.path().join("emoji.zip");
        let mut writer = {
            let path = path.clone();
            blocking(move || Writer::create(&path)).await?.map_err(pack_error)?
        };
        for emoji in &emojis {
            // The picture goes out as the media domain rendered it: decoded
            // and encoded again as WebP, which leaves the pixels and nothing
            // else. The upload itself may carry EXIF — where and when a
            // photo was taken — and a pack is made to be passed around.
            let picture = emoji.image();
            let image = match rendition_key(core, &picture.url) {
                Some(key) => match core.storage.get(Bucket::Media, &key).await {
                    Ok(bytes) => Some(bytes),
                    Err(error) => {
                        tracing::warn!(emoji = %emoji.name, %error, "picture of an emoji could not be read for export");
                        None
                    }
                },
                None => None,
            };
            // A picture that cannot be read is listed without a file, as the
            // format provides for; it does not sink the export.
            let exported = ExportedEmoji {
                id: emoji.id.to_string(),
                name: emoji.name.clone(),
                category: emoji.category.clone(),
                aliases: emoji.aliases.clone(),
                license: emoji.license.clone(),
                sensitive: emoji.sensitive,
                local_only: emoji.local_only,
                content_type: RENDITION_CONTENT_TYPE.to_owned(),
                url: picture.url,
                updated_at: emoji.updated_at.to_rfc3339(),
            };
            writer = blocking(move || {
                let mut writer = writer;
                writer.add(exported, image.as_deref())?;
                Ok::<_, PackError>(writer)
            })
            .await?
            .map_err(pack_error)?;
        }
        let host = core.cfg.public_host();
        let exported_at = now().to_rfc3339();
        let emoji_count = blocking(move || writer.finish(&host, &exported_at)).await?.map_err(pack_error)?;
        let size_bytes = tokio::fs::metadata(&path).await?.len() as i64;
        let object_key = object_key(export.id);
        core.storage.put_file(Bucket::Private, &object_key, &path, "application/zip", None).await?;
        Ok(Written { object_key, size_bytes, emoji_count: emoji_count as i32 })
    }

    /// The worker's periodic pass over exports: deletes the files past their
    /// retention and gives up on exports whose job never finished.
    pub(crate) async fn sweep_exports(&self) -> AppResult<()> {
        let db = &self.core.db;
        let expired: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT id, object_key FROM emoji_exports WHERE status = 3 AND expires_at < now() LIMIT 100",
        )
        .fetch_all(db)
        .await?;
        for (export_id, object_key) in expired {
            self.core.storage.delete(Bucket::Private, &object_key).await?;
            sqlx::query("UPDATE emoji_exports SET status = 5 WHERE id = $1 AND status = 3")
                .bind(export_id)
                .execute(db)
                .await?;
        }
        // An export whose job started six hours ago and never finished is
        // given up. One that only waits for a worker is left to wait.
        sqlx::query(
            "UPDATE emoji_exports SET status = 4, error = 'INTERNAL', completed_at = now()
             WHERE status = 2 AND started_at < now() - interval '6 hours'",
        )
        .execute(db)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_map_to_the_stored_filter() {
        assert_eq!(Scope::Everything.column(), None);
        assert_eq!(Scope::Uncategorized.column(), Some(""));
        assert_eq!(Scope::Category("どうぶつ".into()).column(), Some("どうぶつ"));
    }

    #[test]
    fn archives_live_under_a_fixed_prefix() {
        let id = new_id();
        assert_eq!(object_key(id), format!("emoji/exports/{id}.zip"));
    }
}
