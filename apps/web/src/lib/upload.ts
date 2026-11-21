import { media } from "@tokuly/api";
import { api } from "#/lib/api";

const mediaClient = () => api().service(media.MediaService);

export interface UploadOptions {
  purpose: media.AssetPurpose;
  /** 0..1 as bytes reach the object store. */
  onProgress?(fraction: number): void;
  signal?: AbortSignal;
}

/** Parts uploaded at the same time for multipart uploads. */
const PARALLEL_PARTS = 3;

function put(url: string, body: Blob, headers: Record<string, string>, onProgress: (loaded: number) => void, signal?: AbortSignal): Promise<void> {
  // XMLHttpRequest, because fetch cannot report upload progress.
  return new Promise((resolve, reject) => {
    const request = new XMLHttpRequest();
    request.open("PUT", url);
    for (const [name, value] of Object.entries(headers)) request.setRequestHeader(name, value);
    request.upload.onprogress = (event) => onProgress(event.loaded);
    request.onload = () => (request.status >= 200 && request.status < 300 ? resolve() : reject(new Error(`upload failed (${request.status})`)));
    request.onerror = () => reject(new Error("upload failed (network)"));
    request.onabort = () => reject(new DOMException("upload aborted", "AbortError"));
    signal?.addEventListener("abort", () => request.abort(), { once: true });
    request.send(body);
  });
}

/**
 * Uploads a file straight to object storage and returns the created asset.
 * Small files go up in one request; large ones in parallel parts. The asset
 * usually still needs processing afterwards: see `waitForAsset`.
 */
export async function uploadFile(file: Blob & { name?: string }, options: UploadOptions): Promise<media.Asset> {
  const created = await mediaClient().createUpload(
    { purpose: options.purpose, fileName: file.name ?? "upload", contentType: file.type || "application/octet-stream", sizeBytes: BigInt(file.size) },
    { signal: options.signal },
  );
  try {
    if (created.putUrl) {
      await put(created.putUrl, file, created.putHeaders, (loaded) => options.onProgress?.(loaded / file.size), options.signal);
    } else {
      await uploadParts(file, created, options);
    }
    const completed = await mediaClient().completeUpload({ uploadId: created.uploadId }, { signal: options.signal });
    options.onProgress?.(1);
    return completed.asset!;
  } catch (error) {
    // Do not leave a half-finished upload behind.
    void mediaClient()
      .abortUpload({ uploadId: created.uploadId })
      .catch(() => {});
    throw error;
  }
}

async function uploadParts(file: Blob, created: media.CreateUploadResponse, options: UploadOptions): Promise<void> {
  const partSize = Number(created.partSizeBytes);
  const loaded = new Array<number>(created.partCount).fill(0);
  const report = () => options.onProgress?.(loaded.reduce((sum, bytes) => sum + bytes, 0) / file.size);
  let next = 1;

  const worker = async () => {
    while (next <= created.partCount) {
      const partNumber = next++;
      const { parts } = await mediaClient().getUploadParts({ uploadId: created.uploadId, partNumbers: [partNumber] }, { signal: options.signal });
      const url = parts[0]?.putUrl;
      if (!url) throw new Error(`no upload URL for part ${partNumber}`);
      const chunk = file.slice((partNumber - 1) * partSize, partNumber * partSize);
      await put(
        url,
        chunk,
        {},
        (bytes) => {
          loaded[partNumber - 1] = bytes;
          report();
        },
        options.signal,
      );
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
export async function waitForAsset(assetId: string, onUpdate?: (asset: media.Asset) => void, signal?: AbortSignal): Promise<media.Asset> {
  for await (const update of mediaClient().watchAsset({ assetId }, { signal })) {
    const asset = update.asset;
    if (!asset) continue;
    onUpdate?.(asset);
    if (asset.status === media.AssetStatus.READY) return asset;
    if (asset.status === media.AssetStatus.FAILED) throw new Error(asset.error || "PROCESSING_FAILED");
  }
  throw new Error("PROCESSING_FAILED");
}

/** Upload and wait until the asset is usable: the common case for images. */
export async function uploadAndProcess(file: Blob & { name?: string }, options: UploadOptions): Promise<media.Asset> {
  const asset = await uploadFile(file, options);
  return asset.status === media.AssetStatus.READY ? asset : waitForAsset(asset.id, undefined, options.signal);
}
