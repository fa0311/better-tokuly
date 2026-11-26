//! The run's two object storage buckets, `tokuly-test-<started>-<id>-media`
//! and `…-private`. Objects keys carry nothing that tells a test's upload
//! from a dev server's, so a run gets buckets of its own instead of sharing
//! `tokuly-media` and `tokuly-private`.
use aws_sdk_s3::{
    Client,
    config::{BehaviorVersion, Credentials, Region},
};
use tokuly_core::{config::Config, storage::Storage};

use crate::run::{self, Run};

const HEAD: &str = "tokuly-test-";

/// The run's media and private bucket.
pub(crate) fn names(run: &Run) -> [String; 2] {
    let tag = run.name(HEAD, '-');
    [format!("{tag}-media"), format!("{tag}-private")]
}

fn client(cfg: &Config) -> Client {
    let credentials = Credentials::new(&cfg.s3_access_key, &cfg.s3_secret_key, None, None, "tokuly-testkit");
    let conf = aws_sdk_s3::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(cfg.s3_region.clone()))
        .endpoint_url(&cfg.s3_endpoint)
        .credentials_provider(credentials)
        .force_path_style(true)
        .build();
    Client::from_conf(conf)
}

/// Creates the buckets the way the server does (public media, CORS on
/// private) and returns `cfg` pointing at them; `None` when the object store
/// is not reachable (tests that upload then fail with a connection error).
pub(crate) async fn create(cfg: &Config, run: &Run) -> Option<Config> {
    let [media, private] = names(run);
    let mut cfg = cfg.clone();
    cfg.s3_media_bucket = media;
    cfg.s3_private_bucket = private;
    Storage::new(&cfg).ensure_buckets().await.ok()?;
    Some(cfg)
}

/// Deletes a bucket with everything in it; a missing bucket is fine.
async fn delete_bucket(client: &Client, bucket: &str) -> anyhow::Result<()> {
    if client
        .head_bucket()
        .bucket(bucket)
        .send()
        .await
        .is_err_and(|e| e.as_service_error().is_some_and(|e| e.is_not_found()))
    {
        return Ok(());
    }
    let mut token: Option<String> = None;
    loop {
        let page = client.list_objects_v2().bucket(bucket).set_continuation_token(token).send().await?;
        for key in page.contents().iter().filter_map(|object| object.key()) {
            client.delete_object().bucket(bucket).key(key).send().await?;
        }
        token = page.next_continuation_token().map(str::to_owned);
        if token.is_none() {
            break;
        }
    }
    // Uploads a test started and never completed.
    let uploads = client.list_multipart_uploads().bucket(bucket).send().await?;
    for upload in uploads.uploads() {
        if let (Some(key), Some(id)) = (upload.key(), upload.upload_id()) {
            client.abort_multipart_upload().bucket(bucket).key(key).upload_id(id).send().await?;
        }
    }
    client.delete_bucket().bucket(bucket).send().await?;
    Ok(())
}

pub(crate) async fn remove(cfg: &Config, run: &Run) -> anyhow::Result<()> {
    let client = client(cfg);
    for bucket in names(run) {
        delete_bucket(&client, &bucket).await?;
    }
    Ok(())
}

/// Deletes the buckets of stale runs; returns how many.
pub(crate) async fn sweep(cfg: &Config, now: u64) -> anyhow::Result<usize> {
    let client = client(cfg);
    let listed = client.list_buckets().send().await?;
    let mut deleted = 0;
    for bucket in listed.buckets().iter().filter_map(|b| b.name()).filter(|name| run::is_stale(name, HEAD, '-', now)) {
        if delete_bucket(&client, bucket).await.is_ok() {
            deleted += 1;
        }
    }
    Ok(deleted)
}

#[cfg(test)]
pub(crate) mod tests {
    use tokuly_core::storage::Bucket;

    use super::*;
    use crate::{env::base_config, run::STALE_AFTER};

    pub(crate) async fn exists(cfg: &Config, bucket: &str) -> bool {
        client(cfg).head_bucket().bucket(bucket).send().await.is_ok()
    }

    async fn create_with_objects(cfg: &Config, run: &Run) -> Config {
        let cfg = create(cfg, run).await.expect("the object store is reachable");
        let storage = Storage::new(&cfg);
        for bucket in [Bucket::Media, Bucket::Private] {
            storage.put(bucket, "a/b.txt", bytes::Bytes::from_static(b"left behind"), "text/plain").await.expect("put");
        }
        storage.create_multipart(Bucket::Private, "upload.bin", "application/octet-stream").await.expect("multipart");
        cfg
    }

    #[tokio::test]
    async fn a_run_removes_its_buckets_and_their_objects() {
        let base = base_config();
        let run = Run::new();
        let cfg = create_with_objects(&base, &run).await;
        assert!(exists(&base, &cfg.s3_media_bucket).await);
        remove(&base, &run).await.unwrap();
        for bucket in names(&run) {
            assert!(!exists(&base, &bucket).await, "{bucket} was deleted");
        }
        // Removing twice (a failed setup, then exit) is fine.
        remove(&base, &run).await.unwrap();
    }

    #[tokio::test]
    async fn the_sweep_takes_only_stale_test_buckets() {
        let base = base_config();
        let now = run::unix_now();
        let stale = Run::started_at(now - STALE_AFTER.as_secs() - 60);
        let live = Run::started_at(now - 60);
        create_with_objects(&base, &stale).await;
        create_with_objects(&base, &live).await;
        sweep(&base, now).await.unwrap();
        for bucket in names(&stale) {
            assert!(!exists(&base, &bucket).await, "{bucket} was swept");
        }
        for bucket in names(&live) {
            assert!(exists(&base, &bucket).await, "{bucket} was kept");
        }
        // The shared buckets of the dev API are not testkit's.
        assert!(!run::is_stale(&base.s3_media_bucket, HEAD, '-', now));
        remove(&base, &live).await.unwrap();
    }
}
