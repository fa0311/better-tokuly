//! Where media lives in object storage, and the few storage operations the
//! upload path needs beyond `core.storage`.
//!
//! `core.storage` covers plain reads, writes and multipart bookkeeping. Direct
//! browser uploads need more control over what a presigned URL allows:
//!
//! - upload URLs are signed together with the exact `Content-Length`, so a
//!   URL issued for a 2 MB avatar cannot be used to store 50 GB;
//! - download URLs of private files force `Content-Disposition: attachment`,
//!   the detected content type and `Cache-Control: private, no-store`, so an
//!   uploaded HTML file can never render in the storage origin and no shared
//!   cache keeps a copy;
//! - type detection reads only the first bytes of an object, and checksums
//!   are computed on a stream, so neither loads a large upload into memory.
use std::{sync::OnceLock, time::Duration};

use aws_sdk_s3::{
    Client,
    config::{BehaviorVersion, Credentials, Region},
    presigning::PresigningConfig,
    primitives::{ByteStream, Length},
};
use bytes::Bytes;
use sha2::{Digest as _, Sha256};
use tokuly_core::{config::Config, prelude::*, storage::Bucket};

/// `Cache-Control` of everything written to the media bucket: keys are
/// content-addressed or scoped to an asset that never changes once READY.
pub const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// `Cache-Control` of every download of a private file.
pub const PRIVATE_NO_STORE: &str = "private, no-store";

/// Part size for originals the server itself stores in parts.
const SERVER_PART_BYTES: u64 = 64 * 1024 * 1024;

/// Key of an upload's original bytes in the private bucket.
pub fn original_key(asset_id: Uuid, extension: &str) -> String {
    if extension.is_empty() {
        format!("originals/{asset_id}/source")
    } else {
        format!("originals/{asset_id}/source.{extension}")
    }
}

/// Where a single-PUT upload is kept once it is complete. The upload URL
/// stays valid for a while after completion and writes `original_key`
/// only, so the bytes that were checked cannot be replaced through it.
/// `attempt` is unique per completion attempt: two completions at once each
/// check their own copy, and only the winner's is kept.
pub fn stored_key(asset_id: Uuid, attempt: Uuid, extension: &str) -> String {
    let attempt = attempt.simple();
    if extension.is_empty() {
        format!("originals/{asset_id}/stored-{attempt}")
    } else {
        format!("originals/{asset_id}/stored-{attempt}.{extension}")
    }
}

pub fn original_prefix(asset_id: Uuid) -> String {
    format!("originals/{asset_id}/")
}

/// Key of one WebP variant of an image. The content hash makes it immutable.
pub fn image_key(asset_id: Uuid, size: u32, hash: &str) -> String {
    format!("img/{asset_id}/{size}-{hash}.webp")
}

pub fn animation_key(asset_id: Uuid, hash: &str) -> String {
    format!("anim/{asset_id}/{hash}.webp")
}

pub fn video_prefix(asset_id: Uuid) -> String {
    format!("video/{asset_id}/")
}

pub fn audio_prefix(asset_id: Uuid) -> String {
    format!("audio/{asset_id}/")
}

pub fn public_file_key(asset_id: Uuid, hash: &str, extension: &str) -> String {
    format!("file/{asset_id}/{hash}.{extension}")
}

/// Every media-bucket prefix an asset may have written to.
pub fn public_prefixes(asset_id: Uuid) -> [String; 5] {
    [
        format!("img/{asset_id}/"),
        format!("anim/{asset_id}/"),
        video_prefix(asset_id),
        audio_prefix(asset_id),
        format!("file/{asset_id}/"),
    ]
}

/// First 16 hex characters of the SHA-256 of `bytes`: the content address
/// used in object names.
pub fn content_hash(bytes: &[u8]) -> String {
    hex::encode(&Sha256::digest(bytes)[..8])
}

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Signs URLs for the private bucket. Signing is pure computation — no
/// connection is ever opened — so one process-wide instance ([`presigner`])
/// serves every caller, including the ones that only have a `Core`.
pub struct Presigner {
    client: Client,
    bucket: String,
}

/// Reads and writes originals in the private bucket, for the operations
/// `core.storage` does not offer. Owns network connections, so it belongs to
/// whoever uses it (the `Media` instance, or one server-side ingest).
pub struct OriginalStore {
    client: Client,
    bucket: String,
}

fn client(endpoint: &str, cfg: &Config) -> Client {
    let credentials = Credentials::new(&cfg.s3_access_key, &cfg.s3_secret_key, None, None, "tokuly");
    let conf = aws_sdk_s3::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(cfg.s3_region.clone()))
        .endpoint_url(endpoint)
        .credentials_provider(credentials)
        .force_path_style(true)
        .build();
    Client::from_conf(conf)
}

fn s3_err(op: &str, err: impl std::fmt::Debug) -> AppError {
    AppError::internal(anyhow::anyhow!("object storage {op} failed: {err:?}"))
}

fn presigning(ttl: Duration) -> AppResult<PresigningConfig> {
    PresigningConfig::expires_in(ttl).map_err(|e| s3_err("presign", e))
}

/// The process-wide presigner. A process talks to exactly one object store,
/// so it is built once, from the first `Core` that asks.
pub fn presigner(core: &Core) -> &'static Presigner {
    static PRESIGNER: OnceLock<Presigner> = OnceLock::new();
    PRESIGNER.get_or_init(|| Presigner {
        // Signatures cover the host, so URLs handed to clients are signed
        // for the endpoint clients use.
        client: client(core.cfg.s3_public_endpoint(), &core.cfg),
        bucket: core.cfg.s3_private_bucket.clone(),
    })
}

impl Presigner {
    /// URL for uploading exactly `content_length` bytes of `content_type` in one PUT.
    pub async fn presign_put(&self, key: &str, content_type: &str, content_length: i64, ttl: Duration) -> AppResult<String> {
        let request = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .content_length(content_length)
            .presigned(presigning(ttl)?)
            .await
            .map_err(|e| s3_err("presign put", e))?;
        Ok(request.uri().to_owned())
    }

    /// URL for uploading one part of exactly `content_length` bytes.
    pub async fn presign_part(&self, key: &str, upload_id: &str, part_number: i32, content_length: i64, ttl: Duration) -> AppResult<String> {
        let request = self
            .client
            .upload_part()
            .bucket(&self.bucket)
            .key(key)
            .upload_id(upload_id)
            .part_number(part_number)
            .content_length(content_length)
            .presigned(presigning(ttl)?)
            .await
            .map_err(|e| s3_err("presign part", e))?;
        Ok(request.uri().to_owned())
    }

    /// URL that downloads a private file under its original name. The
    /// response is always an attachment of the given (detected) type.
    pub async fn presign_download(&self, key: &str, file_name: &str, content_type: &str, ttl: Duration) -> AppResult<String> {
        let request = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .response_content_disposition(attachment_disposition(file_name))
            .response_content_type(content_type)
            // Whatever the uploader sent along with the PUT, a private file
            // is never kept by a shared cache (or by the browser).
            .response_cache_control(PRIVATE_NO_STORE)
            .presigned(presigning(ttl)?)
            .await
            .map_err(|e| s3_err("presign get", e))?;
        Ok(request.uri().to_owned())
    }

}

impl OriginalStore {
    pub fn new(cfg: &Config) -> Self {
        Self { client: client(&cfg.s3_endpoint, cfg), bucket: cfg.s3_private_bucket.clone() }
    }

    /// The first `max_bytes` of an object (all of it when it is shorter).
    pub async fn read_prefix(&self, key: &str, max_bytes: usize) -> AppResult<Bytes> {
        let out = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .range(format!("bytes=0-{}", max_bytes.saturating_sub(1)))
            .send()
            .await
            .map_err(|e| s3_err("ranged get", e))?;
        let data = out.body.collect().await.map_err(|e| s3_err("read", e))?;
        Ok(data.into_bytes())
    }

    /// Stores a local file of any size as an original. `core.storage.put_file`
    /// sends one request, which object stores cap at 5 GB; recordings of long
    /// live streams are larger, so big files go up in parts.
    pub async fn put_file(&self, core: &Core, key: &str, path: &std::path::Path, content_type: &str) -> AppResult<()> {
        let size = tokio::fs::metadata(path).await.map_err(|e| s3_err("read file", e))?.len();
        if size <= SERVER_PART_BYTES {
            return core.storage.put_file(Bucket::Private, key, path, content_type, None).await;
        }
        let upload_id = core.storage.create_multipart(Bucket::Private, key, content_type).await?;
        let upload = async {
            let mut parts = Vec::new();
            let mut offset = 0u64;
            while offset < size {
                let length = SERVER_PART_BYTES.min(size - offset);
                let number = parts.len() as i32 + 1;
                let body = ByteStream::read_from().path(path).offset(offset).length(Length::Exact(length)).build().await.map_err(|e| s3_err("read file", e))?;
                let part = self
                    .client
                    .upload_part()
                    .bucket(&self.bucket)
                    .key(key)
                    .upload_id(&upload_id)
                    .part_number(number)
                    .body(body)
                    .send()
                    .await
                    .map_err(|e| s3_err("upload part", e))?;
                parts.push((number, part.e_tag().unwrap_or_default().to_owned()));
                offset += length;
            }
            core.storage.complete_multipart(Bucket::Private, key, &upload_id, parts).await
        };
        let result = upload.await;
        if result.is_err() {
            // Do not leave the parts behind; they are invisible but billed.
            let _ = core.storage.abort_multipart(Bucket::Private, key, &upload_id).await;
        }
        result
    }

    /// Lowercase hex SHA-256 of an object, computed while streaming it.
    pub async fn sha256(&self, key: &str) -> AppResult<String> {
        let out = self.client.get_object().bucket(&self.bucket).key(key).send().await.map_err(|e| s3_err("get", e))?;
        let mut body = out.body;
        let mut hasher = Sha256::new();
        while let Some(chunk) = body.try_next().await.map_err(|e| s3_err("read", e))? {
            hasher.update(&chunk);
        }
        Ok(hex::encode(hasher.finalize()))
    }
}

/// `Content-Disposition` value that downloads under `file_name`: an ASCII
/// fallback plus the exact name in RFC 5987 form for everything else.
pub fn attachment_disposition(file_name: &str) -> String {
    let fallback: String = file_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ' | '(' | ')') { c } else { '_' })
        .collect();
    let mut encoded = String::with_capacity(file_name.len() * 3);
    for byte in file_name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

/// Lowercase hex SHA-256 of a local file (blocking; run on the blocking pool).
pub fn sha256_file(path: &std::path::Path) -> std::io::Result<String> {
    use std::io::Read as _;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispositions_survive_hostile_file_names() {
        assert_eq!(attachment_disposition("report.pdf"), "attachment; filename=\"report.pdf\"; filename*=UTF-8''report.pdf");
        let value = attachment_disposition("写真 \"1\".png\r\nX-Evil: 1");
        assert!(value.starts_with("attachment; filename=\"__ _1_.png__X-Evil_ 1\"; filename*=UTF-8''%E5%86%99"));
        assert!(!value.contains('\r') && !value.contains('\n'));
    }

    #[test]
    fn keys_are_scoped_to_the_asset() {
        let id = Uuid::nil();
        assert_eq!(original_key(id, "mp4"), format!("originals/{id}/source.mp4"));
        assert_eq!(original_key(id, ""), format!("originals/{id}/source"));
        assert!(original_key(id, "mp4").starts_with(&original_prefix(id)));
        assert!(stored_key(id, Uuid::max(), "mp4").starts_with(&original_prefix(id)));
        assert_ne!(stored_key(id, Uuid::nil(), "mp4"), original_key(id, "mp4"));
        assert_ne!(stored_key(id, Uuid::nil(), "mp4"), stored_key(id, Uuid::max(), "mp4"));
        assert_eq!(image_key(id, 512, "abcd"), format!("img/{id}/512-abcd.webp"));
        for prefix in public_prefixes(id) {
            assert!(prefix.ends_with(&format!("{id}/")), "{prefix}");
        }
        assert_eq!(content_hash(b"hello").len(), 16);
        assert_ne!(content_hash(b"hello"), content_hash(b"hellp"));
    }
}
