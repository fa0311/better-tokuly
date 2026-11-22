//! Object storage over the S3 API (RustFS in the default stack).
//!
//! Two buckets:
//! - the **media** bucket holds processed files served publicly through the
//!   edge at [`Storage::media_url`];
//! - the **private** bucket holds original uploads and anything that needs a
//!   presigned URL to read.
//!
//! Presigned URLs are created with the *public* endpoint so the signature
//! matches the host the browser actually talks to.
use std::time::Duration;

use aws_sdk_s3::{
    Client,
    config::{BehaviorVersion, Credentials, Region},
    presigning::PresigningConfig,
    primitives::ByteStream,
    types::{CompletedMultipartUpload, CompletedPart, CorsConfiguration, CorsRule},
};
use bytes::Bytes;

use crate::{
    config::Config,
    error::{AppError, AppResult},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bucket {
    Media,
    Private,
}

#[derive(Clone)]
pub struct Storage {
    internal: Client,
    public: Client,
    media_bucket: String,
    private_bucket: String,
    media_url: String,
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

fn s3_err(op: &str, err: impl std::fmt::Display) -> AppError {
    AppError::internal(anyhow::anyhow!("object storage {op} failed: {err}"))
}

impl Storage {
    pub fn new(cfg: &Config) -> Self {
        Self {
            internal: client(&cfg.s3_endpoint, cfg),
            public: client(cfg.s3_public_endpoint(), cfg),
            media_bucket: cfg.s3_media_bucket.clone(),
            private_bucket: cfg.s3_private_bucket.clone(),
            media_url: cfg.media_url(),
        }
    }

    pub fn bucket_name(&self, bucket: Bucket) -> &str {
        match bucket {
            Bucket::Media => &self.media_bucket,
            Bucket::Private => &self.private_bucket,
        }
    }

    /// Public URL of an object in the media bucket.
    pub fn media_url(&self, key: &str) -> String {
        format!("{}/{}", self.media_url, key.trim_start_matches('/'))
    }

    /// Creates both buckets and makes the media bucket world-readable. Idempotent.
    pub async fn ensure_buckets(&self) -> anyhow::Result<()> {
        for name in [&self.media_bucket, &self.private_bucket] {
            if self.internal.head_bucket().bucket(name).send().await.is_err() {
                if let Err(e) = self.internal.create_bucket().bucket(name).send().await {
                    let text = format!("{e:?}");
                    if !text.contains("BucketAlreadyOwnedByYou") && !text.contains("BucketAlreadyExists") {
                        anyhow::bail!("cannot create bucket {name}: {text}");
                    }
                }
            }
        }
        let policy = serde_json::json!({
            "Version": "2012-10-17",
            "Statement": [{
                "Effect": "Allow",
                "Principal": {"AWS": ["*"]},
                "Action": ["s3:GetObject"],
                "Resource": [format!("arn:aws:s3:::{}/*", self.media_bucket)]
            }]
        });
        self.internal
            .put_bucket_policy()
            .bucket(&self.media_bucket)
            .policy(policy.to_string())
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("cannot set public-read policy on {}: {e:?}", self.media_bucket))?;
        self.allow_browser_uploads().await
    }

    /// Lets browsers upload straight to presigned URLs on the private bucket,
    /// which is a cross-origin request from the web app. The presigned URL is
    /// the credential, so any origin may use it; `ETag` must be readable for
    /// multipart uploads.
    async fn allow_browser_uploads(&self) -> anyhow::Result<()> {
        let rule = CorsRule::builder()
            .allowed_origins("*")
            .allowed_methods("PUT")
            .allowed_methods("GET")
            .allowed_methods("HEAD")
            .allowed_headers("*")
            .expose_headers("ETag")
            .max_age_seconds(3600)
            .build()
            .map_err(|e| anyhow::anyhow!("invalid CORS rule: {e}"))?;
        let cors = CorsConfiguration::builder()
            .cors_rules(rule)
            .build()
            .map_err(|e| anyhow::anyhow!("invalid CORS configuration: {e}"))?;
        let result = self.internal.put_bucket_cors().bucket(&self.private_bucket).cors_configuration(cors).send().await;
        if let Err(e) = result {
            // Some S3-compatible stores configure CORS globally instead; direct
            // browser uploads then depend on that setting.
            tracing::warn!(error = ?e, bucket = %self.private_bucket, "could not set bucket CORS; configure it on the object store");
        }
        Ok(())
    }

    pub async fn put(&self, bucket: Bucket, key: &str, body: impl Into<Bytes>, content_type: &str) -> AppResult<()> {
        self.put_with_cache(bucket, key, body, content_type, None).await
    }

    /// Uploads an object, optionally with a `Cache-Control` header (e.g.
    /// `public, max-age=31536000, immutable` for content-addressed files).
    pub async fn put_with_cache(
        &self,
        bucket: Bucket,
        key: &str,
        body: impl Into<Bytes>,
        content_type: &str,
        cache_control: Option<&str>,
    ) -> AppResult<()> {
        let mut req = self
            .internal
            .put_object()
            .bucket(self.bucket_name(bucket))
            .key(key)
            .content_type(content_type)
            .body(ByteStream::from(body.into()));
        if let Some(cc) = cache_control {
            req = req.cache_control(cc);
        }
        req.send().await.map_err(|e| s3_err("put", format!("{e:?}")))?;
        Ok(())
    }

    /// Uploads a local file (streamed, not loaded into memory).
    pub async fn put_file(
        &self,
        bucket: Bucket,
        key: &str,
        path: &std::path::Path,
        content_type: &str,
        cache_control: Option<&str>,
    ) -> AppResult<()> {
        let body = ByteStream::from_path(path).await.map_err(|e| s3_err("read file", e))?;
        let mut req =
            self.internal.put_object().bucket(self.bucket_name(bucket)).key(key).content_type(content_type).body(body);
        if let Some(cc) = cache_control {
            req = req.cache_control(cc);
        }
        req.send().await.map_err(|e| s3_err("put file", format!("{e:?}")))?;
        Ok(())
    }

    pub async fn get(&self, bucket: Bucket, key: &str) -> AppResult<Bytes> {
        let out = self
            .internal
            .get_object()
            .bucket(self.bucket_name(bucket))
            .key(key)
            .send()
            .await
            .map_err(|e| s3_err("get", format!("{e:?}")))?;
        let data = out.body.collect().await.map_err(|e| s3_err("read", e))?;
        Ok(data.into_bytes())
    }

    /// Downloads an object to a local file (streamed).
    pub async fn download(&self, bucket: Bucket, key: &str, path: &std::path::Path) -> AppResult<u64> {
        use tokio::io::AsyncWriteExt as _;
        let out = self
            .internal
            .get_object()
            .bucket(self.bucket_name(bucket))
            .key(key)
            .send()
            .await
            .map_err(|e| s3_err("get", format!("{e:?}")))?;
        let mut body = out.body;
        let mut file = tokio::fs::File::create(path).await.map_err(|e| s3_err("create file", e))?;
        let mut total = 0u64;
        while let Some(chunk) = body.try_next().await.map_err(|e| s3_err("read", e))? {
            total += chunk.len() as u64;
            file.write_all(&chunk).await.map_err(|e| s3_err("write file", e))?;
        }
        file.flush().await.map_err(|e| s3_err("flush file", e))?;
        Ok(total)
    }

    /// Size in bytes, or `None` when the object does not exist.
    pub async fn head(&self, bucket: Bucket, key: &str) -> AppResult<Option<i64>> {
        match self.internal.head_object().bucket(self.bucket_name(bucket)).key(key).send().await {
            Ok(out) => Ok(Some(out.content_length().unwrap_or(0))),
            Err(e) => {
                if e.as_service_error().is_some_and(|s| s.is_not_found()) {
                    Ok(None)
                } else {
                    Err(s3_err("head", format!("{e:?}")))
                }
            }
        }
    }

    pub async fn delete(&self, bucket: Bucket, key: &str) -> AppResult<()> {
        self.internal
            .delete_object()
            .bucket(self.bucket_name(bucket))
            .key(key)
            .send()
            .await
            .map_err(|e| s3_err("delete", format!("{e:?}")))?;
        Ok(())
    }

    /// Deletes every object under a key prefix (e.g. all renditions of an asset).
    pub async fn delete_prefix(&self, bucket: Bucket, prefix: &str) -> AppResult<u64> {
        let name = self.bucket_name(bucket);
        let mut deleted = 0;
        let mut token: Option<String> = None;
        loop {
            let page = self
                .internal
                .list_objects_v2()
                .bucket(name)
                .prefix(prefix)
                .set_continuation_token(token.clone())
                .send()
                .await
                .map_err(|e| s3_err("list", format!("{e:?}")))?;
            for object in page.contents() {
                if let Some(key) = object.key() {
                    self.internal
                        .delete_object()
                        .bucket(name)
                        .key(key)
                        .send()
                        .await
                        .map_err(|e| s3_err("delete", format!("{e:?}")))?;
                    deleted += 1;
                }
            }
            token = page.next_continuation_token().map(str::to_owned);
            if token.is_none() {
                break;
            }
        }
        Ok(deleted)
    }

    /// Server-side copy between buckets/keys.
    pub async fn copy(&self, from: (Bucket, &str), to: (Bucket, &str)) -> AppResult<()> {
        self.internal
            .copy_object()
            .copy_source(format!("{}/{}", self.bucket_name(from.0), from.1))
            .bucket(self.bucket_name(to.0))
            .key(to.1)
            .send()
            .await
            .map_err(|e| s3_err("copy", format!("{e:?}")))?;
        Ok(())
    }

    fn presign_config(ttl: Duration) -> AppResult<PresigningConfig> {
        PresigningConfig::expires_in(ttl).map_err(|e| s3_err("presign", e))
    }

    /// Browser-usable URL for a single-request upload.
    pub async fn presign_put(&self, bucket: Bucket, key: &str, content_type: &str, ttl: Duration) -> AppResult<String> {
        let req = self
            .public
            .put_object()
            .bucket(self.bucket_name(bucket))
            .key(key)
            .content_type(content_type)
            .presigned(Self::presign_config(ttl)?)
            .await
            .map_err(|e| s3_err("presign put", format!("{e:?}")))?;
        Ok(req.uri().to_string())
    }

    /// Browser-usable URL for a time-limited download.
    pub async fn presign_get(&self, bucket: Bucket, key: &str, ttl: Duration) -> AppResult<String> {
        let req = self
            .public
            .get_object()
            .bucket(self.bucket_name(bucket))
            .key(key)
            .presigned(Self::presign_config(ttl)?)
            .await
            .map_err(|e| s3_err("presign get", format!("{e:?}")))?;
        Ok(req.uri().to_string())
    }

    // ── multipart uploads ──

    /// Starts a multipart upload and returns its storage upload ID.
    pub async fn create_multipart(&self, bucket: Bucket, key: &str, content_type: &str) -> AppResult<String> {
        let out = self
            .internal
            .create_multipart_upload()
            .bucket(self.bucket_name(bucket))
            .key(key)
            .content_type(content_type)
            .send()
            .await
            .map_err(|e| s3_err("create multipart", format!("{e:?}")))?;
        out.upload_id().map(str::to_owned).ok_or_else(|| s3_err("create multipart", "no upload id returned"))
    }

    pub async fn presign_part(
        &self,
        bucket: Bucket,
        key: &str,
        upload_id: &str,
        part_number: i32,
        ttl: Duration,
    ) -> AppResult<String> {
        let req = self
            .public
            .upload_part()
            .bucket(self.bucket_name(bucket))
            .key(key)
            .upload_id(upload_id)
            .part_number(part_number)
            .presigned(Self::presign_config(ttl)?)
            .await
            .map_err(|e| s3_err("presign part", format!("{e:?}")))?;
        Ok(req.uri().to_string())
    }

    /// Part numbers (with ETags) the store already holds for an upload.
    pub async fn list_parts(&self, bucket: Bucket, key: &str, upload_id: &str) -> AppResult<Vec<(i32, String)>> {
        let mut parts = Vec::new();
        let mut marker: Option<String> = None;
        loop {
            let out = self
                .internal
                .list_parts()
                .bucket(self.bucket_name(bucket))
                .key(key)
                .upload_id(upload_id)
                .set_part_number_marker(marker.clone())
                .send()
                .await
                .map_err(|e| s3_err("list parts", format!("{e:?}")))?;
            for part in out.parts() {
                if let (Some(n), Some(etag)) = (part.part_number(), part.e_tag()) {
                    parts.push((n, etag.to_owned()));
                }
            }
            if out.is_truncated().unwrap_or(false) {
                marker = out.next_part_number_marker().map(str::to_owned);
            } else {
                break;
            }
        }
        Ok(parts)
    }

    pub async fn complete_multipart(
        &self,
        bucket: Bucket,
        key: &str,
        upload_id: &str,
        mut parts: Vec<(i32, String)>,
    ) -> AppResult<()> {
        parts.sort_by_key(|(n, _)| *n);
        let completed = CompletedMultipartUpload::builder()
            .set_parts(Some(
                parts
                    .into_iter()
                    .map(|(n, etag)| CompletedPart::builder().part_number(n).e_tag(etag).build())
                    .collect(),
            ))
            .build();
        self.internal
            .complete_multipart_upload()
            .bucket(self.bucket_name(bucket))
            .key(key)
            .upload_id(upload_id)
            .multipart_upload(completed)
            .send()
            .await
            .map_err(|e| s3_err("complete multipart", format!("{e:?}")))?;
        Ok(())
    }

    pub async fn abort_multipart(&self, bucket: Bucket, key: &str, upload_id: &str) -> AppResult<()> {
        self.internal
            .abort_multipart_upload()
            .bucket(self.bucket_name(bucket))
            .key(key)
            .upload_id(upload_id)
            .send()
            .await
            .map_err(|e| s3_err("abort multipart", format!("{e:?}")))?;
        Ok(())
    }
}
