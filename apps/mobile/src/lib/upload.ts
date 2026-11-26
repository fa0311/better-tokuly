import { type Api, media } from "@tokuly/api";
import { AssetFailedError } from "@tokuly/core/errors";
import { api } from "#/lib/api";
import { SERVER_URL } from "#/lib/config";

const mediaClient = (options: { api?: Api }) => (options.api ?? api()).service(media.MediaService);

/** A file on the device (a photo from the library or the camera), as the pickers hand it over. */
export interface LocalFile {
  uri: string;
  name: string;
  type: string;
}

export interface UploadOptions {
  purpose: media.AssetPurpose;
  /** 0..1 as bytes reach the object store. */
  onProgress?(fraction: number): void;
  /** The account the upload belongs to (`Account.api`); the active one when left out. */
  api?: Api;
}

const PARALLEL_PARTS = 3;

/** An upload URL the server gave as a path is on the app's own server. */
const absolute = (url: string) => (url.startsWith("/") ? `${SERVER_URL}${url}` : url);

/** XMLHttpRequest, because fetch cannot report upload progress. */
function put(
  url: string,
  body: Blob,
  headers: Record<string, string>,
  onProgress: (loaded: number) => void,
): Promise<void> {
  return new Promise((resolve, reject) => {
    const request = new XMLHttpRequest();
    request.open("PUT", absolute(url));
    for (const [name, value] of Object.entries(headers)) request.setRequestHeader(name, value);
    request.upload.onprogress = (event) => onProgress(event.loaded);
    request.onload = () =>
      request.status >= 200 && request.status < 300
        ? resolve()
        : reject(new Error(`upload failed (${request.status})`));
    request.onerror = () => reject(new Error("upload failed (network)"));
    request.send(body);
  });
}

async function putParts(
  blob: Blob,
  created: media.CreateUploadResponse,
  options: UploadOptions,
): Promise<void> {
  const partSize = Number(created.partSizeBytes);
  const loaded = new Array<number>(created.partCount).fill(0);
  const report = () =>
    options.onProgress?.(loaded.reduce((sum, bytes) => sum + bytes, 0) / blob.size);
  let next = 1;
  const worker = async () => {
    while (next <= created.partCount) {
      const partNumber = next++;
      const { parts } = await mediaClient(options).getUploadParts({
        uploadId: created.uploadId,
        partNumbers: [partNumber],
      });
      const url = parts[0]?.putUrl;
      if (!url) throw new Error(`no upload URL for part ${partNumber}`);
      const chunk = blob.slice((partNumber - 1) * partSize, partNumber * partSize);
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

/** Resolves once the asset finished processing (READY); rejects when it failed. */
async function waitForAsset(assetId: string, options: UploadOptions): Promise<media.Asset> {
  for await (const update of mediaClient(options).watchAsset({ assetId })) {
    const asset = update.asset;
    if (asset?.status === media.AssetStatus.READY) return asset;
    if (asset?.status === media.AssetStatus.FAILED)
      throw new AssetFailedError(asset.error || "PROCESSING_FAILED", asset.maxDurationSeconds);
  }
  throw new AssetFailedError("PROCESSING_FAILED");
}

/**
 * The web's upload flow on the phone: the bytes go straight to object
 * storage (one request, or parts for a large file), then the asset is
 * processed and waited for. A half-finished upload is aborted.
 */
export async function uploadAndProcess(
  file: LocalFile,
  options: UploadOptions,
): Promise<media.Asset> {
  const blob = await (await fetch(file.uri)).blob();
  const created = await mediaClient(options).createUpload({
    purpose: options.purpose,
    fileName: file.name,
    contentType: file.type || blob.type || "application/octet-stream",
    sizeBytes: BigInt(blob.size),
  });
  let asset: media.Asset;
  try {
    if (created.putUrl)
      await put(created.putUrl, blob, created.putHeaders, (bytes) =>
        options.onProgress?.(bytes / blob.size),
      );
    else await putParts(blob, created, options);
    asset = (await mediaClient(options).completeUpload({ uploadId: created.uploadId })).asset!;
    options.onProgress?.(1);
  } catch (error) {
    void mediaClient(options)
      .abortUpload({ uploadId: created.uploadId })
      .catch(() => {});
    throw error;
  }
  return asset.status === media.AssetStatus.READY ? asset : waitForAsset(asset.id, options);
}
