//! `tokuly.media.v1.MediaService`: the RPC surface. Handlers authenticate,
//! parse and delegate; the logic lives in `upload`, `lifecycle`, `assets`
//! and `link_preview`.
use std::time::Duration;

use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult, ServiceStream};
use futures::{Stream, StreamExt as _};
use tokuly_core::{prelude::*, realtime::topic_subject};
use tokuly_proto::media::*;

use crate::{
    Media,
    assets::{self, AssetRow},
    events::asset_topic,
    lifecycle,
    upload::NewUpload,
};

/// Assets returned by one `BatchGetAssets` call.
const MAX_BATCH: usize = 100;
/// Link previews one user may request per minute.
const PREVIEWS_PER_MINUTE: u32 = 60;
/// How often a watch stream re-reads the asset when no push arrives
/// (realtime delivery is best-effort).
const WATCH_POLL: Duration = Duration::from_secs(5);

/// What `viewer` may see of an asset: everything of their own; of others'
/// only finished assets, in their public form.
fn visible_to(row: &AssetRow, viewer: Uuid) -> Option<Asset> {
    if row.owner_id == viewer {
        Some(row.to_proto())
    } else if row.is_ready() {
        Some(row.to_public_proto())
    } else {
        None
    }
}

fn is_settled(row: &AssetRow) -> bool {
    row.is_ready() || row.has_status(AssetStatus::ASSET_STATUS_FAILED)
}

impl MediaService for Media {
    async fn create_upload(&self, ctx: RequestContext, request: ServiceRequest<'_, CreateUploadRequest>) -> ServiceResult<CreateUploadResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let upload = NewUpload {
                purpose: request.purpose.as_known(),
                file_name: request.file_name,
                content_type: request.content_type,
                size_bytes: request.size_bytes,
                sha256: request.sha256,
            };
            ok(self.create_upload(user_id, upload).await?)
        })
        .await
    }

    async fn get_upload_parts(&self, ctx: RequestContext, request: ServiceRequest<'_, GetUploadPartsRequest>) -> ServiceResult<GetUploadPartsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let upload_id = parse_id(request.upload_id)?;
            let part_numbers: Vec<i32> = request.part_numbers.iter().copied().collect();
            ok(self.upload_parts(user_id, upload_id, &part_numbers).await?)
        })
        .await
    }

    async fn complete_upload(&self, ctx: RequestContext, request: ServiceRequest<'_, CompleteUploadRequest>) -> ServiceResult<CompleteUploadResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let upload_id = parse_id(request.upload_id)?;
            let parts: Vec<(i32, String)> = request.parts.iter().map(|part| (part.part_number, part.etag.to_owned())).collect();
            let asset = self.complete_upload(user_id, upload_id, &parts).await?;
            ok(CompleteUploadResponse { asset: asset.to_proto().into(), ..Default::default() })
        })
        .await
    }

    async fn abort_upload(&self, ctx: RequestContext, request: ServiceRequest<'_, AbortUploadRequest>) -> ServiceResult<AbortUploadResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.abort_upload(user_id, parse_id(request.upload_id)?).await?;
            ok(AbortUploadResponse::default())
        })
        .await
    }

    async fn get_asset(&self, ctx: RequestContext, request: ServiceRequest<'_, GetAssetRequest>) -> ServiceResult<GetAssetResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let row = assets::find(&self.core, parse_id(request.asset_id)?).await?;
            let asset = row.and_then(|row| visible_to(&row, user_id)).or_not_found("ASSET_NOT_FOUND", "asset not found")?;
            ok(GetAssetResponse { asset: asset.into(), ..Default::default() })
        })
        .await
    }

    async fn batch_get_assets(&self, ctx: RequestContext, request: ServiceRequest<'_, BatchGetAssetsRequest>) -> ServiceResult<BatchGetAssetsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            ensure!(request.asset_ids.len() <= MAX_BATCH, invalid, "TOO_MANY_ASSETS", "at most {MAX_BATCH} assets per call");
            let ids = request.asset_ids.iter().map(|id| parse_id(id)).collect::<AppResult<Vec<Uuid>>>()?;
            let rows = assets::find_many(&self.core, &ids).await?;
            let assets = rows.iter().filter_map(|row| visible_to(row, user_id)).collect();
            ok(BatchGetAssetsResponse { assets, ..Default::default() })
        })
        .await
    }

    async fn watch_asset(&self, ctx: RequestContext, request: ServiceRequest<'_, WatchAssetRequest>) -> ServiceResult<ServiceStream<WatchAssetResponse>> {
        let watch = handle(async {
            let user_id = ctx.user_id()?;
            let asset_id = parse_id(request.asset_id)?;
            // Subscribe before reading, so a change between the read and the
            // subscription cannot be missed.
            let updates = self.core.bus.subscribe(topic_subject(&asset_topic(asset_id))).await?;
            let row = assets::find(&self.core, asset_id)
                .await?
                .filter(|row| row.owner_id == user_id)
                .or_not_found("ASSET_NOT_FOUND", "asset not found")?;
            Ok(Watch { core: self.core.clone(), asset_id, updates: Some(updates), first: Some(row), last_sent: None, settled: false })
        })
        .await?;
        Response::stream_ok(watch.into_stream())
    }

    async fn delete_asset(&self, ctx: RequestContext, request: ServiceRequest<'_, DeleteAssetRequest>) -> ServiceResult<DeleteAssetResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let deleted = lifecycle::delete(&self.core, parse_id(request.asset_id)?, Some(user_id)).await?;
            ensure!(deleted, not_found, "ASSET_NOT_FOUND", "asset not found");
            ok(DeleteAssetResponse::default())
        })
        .await
    }

    async fn get_link_preview(&self, ctx: RequestContext, request: ServiceRequest<'_, GetLinkPreviewRequest>) -> ServiceResult<GetLinkPreviewResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.core.cache.rate_limit(&format!("media:preview:{user_id}"), PREVIEWS_PER_MINUTE, Duration::from_secs(60)).await?;
            let preview = self.previews.preview(&self.core, request.url).await?;
            ok(GetLinkPreviewResponse { preview: preview.into(), ..Default::default() })
        })
        .await
    }
}

/// State of one `WatchAsset` stream.
struct Watch {
    core: Core,
    asset_id: Uuid,
    /// Wake-ups from the asset's realtime topic; `None` once the bus
    /// subscription ended (the stream then only polls).
    updates: Option<async_nats::Subscriber>,
    /// The state read when the stream was opened; sent first.
    first: Option<AssetRow>,
    /// `(status, progress)` of the last message sent, to skip no-op wake-ups.
    last_sent: Option<(i32, i32)>,
    settled: bool,
}

impl Watch {
    fn into_stream(self) -> impl Stream<Item = Result<WatchAssetResponse, ConnectError>> + Send + 'static {
        futures::stream::unfold(self, |mut watch| async move {
            if watch.settled {
                return None;
            }
            let row = match watch.first.take() {
                Some(row) => row,
                None => match watch.next_change().await {
                    Ok(Some(row)) => row,
                    // Deleted while being watched, or the server is stopping.
                    Ok(None) => return None,
                    Err(e) => {
                        watch.settled = true;
                        return Some((Err(e.into()), watch));
                    }
                },
            };
            watch.last_sent = Some((row.status, row.progress));
            watch.settled = is_settled(&row);
            let message = WatchAssetResponse { asset: row.to_proto().into(), ..Default::default() };
            Some((Ok(message), watch))
        })
    }

    /// Waits until the asset differs from what was last sent. Pushes on the
    /// asset topic are only wake-ups: the database row is what gets sent.
    async fn next_change(&mut self) -> AppResult<Option<AssetRow>> {
        loop {
            let bus_closed = match &mut self.updates {
                Some(updates) => tokio::select! {
                    _ = self.core.shutdown.cancelled() => return Ok(None),
                    _ = tokio::time::sleep(WATCH_POLL) => false,
                    update = updates.next() => update.is_none(),
                },
                None => tokio::select! {
                    _ = self.core.shutdown.cancelled() => return Ok(None),
                    _ = tokio::time::sleep(WATCH_POLL) => false,
                },
            };
            if bus_closed {
                self.updates = None;
            }
            let Some(row) = assets::find(&self.core, self.asset_id).await? else { return Ok(None) };
            if self.last_sent != Some((row.status, row.progress)) {
                return Ok(Some(row));
            }
        }
    }
}
