import { media } from "@tokuly/api";
import { AssetFailedError } from "../errors";
import type { ClientSource } from "../query";

type MediaClient = ClientSource<typeof media.MediaService>;

/** What an upload sends: a file's bytes, as a `Blob` is (only its size and slices are needed). */
export interface Bytes<B> {
  readonly size: number;
  slice(start: number, end: number): B;
}

/** Sends `body` to a presigned URL with `headers`, reporting the bytes sent so far. */
export type PutBytes<B> = (
  url: string,
  body: B,
  headers: Record<string, string>,
  onProgress: (loaded: number) => void,
) => Promise<void>;

/** The file an upload creates an asset for. */
export interface UploadRequest {
  purpose: media.AssetPurpose;
  fileName: string;
  contentType: string;
}

export interface UploadOptions {
  /** 0..1 as bytes reach the object store. */
  onProgress?(fraction: number): void;
  signal?: AbortSignal;
}

/** Parts uploaded at the same time for multipart uploads. */
const PARALLEL_PARTS = 3;

/**
 * Uploads `body` straight to object storage and returns the created asset.
 * Small files go up in one request; large ones in parallel parts. The asset
 * usually still needs processing afterwards: see `processed`.
 */
export async function uploadBytes<B extends Bytes<B>>(
  client: MediaClient,
  request: UploadRequest,
  body: B,
  put: PutBytes<B>,
  options: UploadOptions = {},
): Promise<media.Asset> {
  const { signal } = options;
  const created = await client().createUpload(
    { ...request, sizeBytes: BigInt(body.size) },
    { signal },
  );
  try {
    if (created.putUrl) {
      await put(created.putUrl, body, created.putHeaders, (loaded) =>
        options.onProgress?.(loaded / body.size),
      );
    } else {
      await putParts(client, body, created, put, options);
    }
    const completed = await client().completeUpload({ uploadId: created.uploadId }, { signal });
    options.onProgress?.(1);
    return completed.asset!;
  } catch (error) {
    // Do not leave a half-finished upload behind.
    void client()
      .abortUpload({ uploadId: created.uploadId })
      .catch(() => {});
    throw error;
  }
}

async function putParts<B extends Bytes<B>>(
  client: MediaClient,
  body: B,
  created: media.CreateUploadResponse,
  put: PutBytes<B>,
  options: UploadOptions,
): Promise<void> {
  const partSize = Number(created.partSizeBytes);
  const loaded = new Array<number>(created.partCount).fill(0);
  const report = () =>
    options.onProgress?.(loaded.reduce((sum, bytes) => sum + bytes, 0) / body.size);
  let next = 1;

  const worker = async () => {
    while (next <= created.partCount) {
      const partNumber = next++;
      const { parts } = await client().getUploadParts(
        { uploadId: created.uploadId, partNumbers: [partNumber] },
        { signal: options.signal },
      );
      const url = parts[0]?.putUrl;
      if (!url) throw new Error(`no upload URL for part ${partNumber}`);
      const start = (partNumber - 1) * partSize;
      const chunk = body.slice(start, Math.min(body.size, start + partSize));
      await put(url, chunk, {}, (bytes) => {
        loaded[partNumber - 1] = bytes;
        report();
      });
      loaded[partNumber - 1] = chunk.size;
      report();
    }
  };
  await Promise.all(Array.from({ length: Math.min(PARALLEL_PARTS, created.partCount) }, worker));
}

/**
 * Resolves once the asset finished processing (READY) and rejects if it
 * failed. `onUpdate` receives progress while it is being processed.
 */
export async function waitForAsset(
  client: MediaClient,
  assetId: string,
  onUpdate?: (asset: media.Asset) => void,
  signal?: AbortSignal,
): Promise<media.Asset> {
  for await (const update of client().watchAsset({ assetId }, { signal })) {
    const asset = update.asset;
    if (!asset) continue;
    onUpdate?.(asset);
    if (asset.status === media.AssetStatus.READY) return asset;
    if (asset.status === media.AssetStatus.FAILED)
      throw new AssetFailedError(asset.error || "PROCESSING_FAILED", asset.maxDurationSeconds);
  }
  throw new AssetFailedError("PROCESSING_FAILED");
}

/** The asset once it is usable: at once when it is READY already, after its processing otherwise. */
export function processed(
  client: MediaClient,
  asset: media.Asset,
  signal?: AbortSignal,
): Promise<media.Asset> {
  return asset.status === media.AssetStatus.READY
    ? Promise.resolve(asset)
    : waitForAsset(client, asset.id, undefined, signal);
}
