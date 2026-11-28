import { media } from "@tokuly/api";
import { AssetFailedError, NetworkError, TransferError } from "@tokuly/core/errors";
import * as uploads from "@tokuly/core/media";
import { api } from "#/lib/api";

export { AssetFailedError };

const mediaClient = () => api().service(media.MediaService);

export interface UploadOptions {
  purpose: media.AssetPurpose;
  /** 0..1 as bytes reach the object store. */
  onProgress?(fraction: number): void;
  signal?: AbortSignal;
}

function put(
  url: string,
  body: Blob,
  headers: Record<string, string>,
  onProgress: (loaded: number) => void,
  signal?: AbortSignal,
): Promise<void> {
  // XMLHttpRequest, because fetch cannot report upload progress.
  return new Promise((resolve, reject) => {
    const request = new XMLHttpRequest();
    request.open("PUT", url);
    for (const [name, value] of Object.entries(headers)) request.setRequestHeader(name, value);
    request.upload.onprogress = (event) => onProgress(event.loaded);
    request.onload = () =>
      request.status >= 200 && request.status < 300
        ? resolve()
        : reject(new TransferError(request.status));
    request.onerror = () => reject(new NetworkError("upload failed (network)"));
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
export async function uploadFile(
  file: Blob & { name?: string },
  options: UploadOptions,
): Promise<media.Asset> {
  return uploads.uploadBytes(
    mediaClient,
    {
      purpose: options.purpose,
      fileName: file.name ?? "upload",
      contentType: file.type || "application/octet-stream",
    },
    file,
    (url, body, headers, onProgress) => put(url, body, headers, onProgress, options.signal),
    options,
  );
}

/**
 * Resolves once the asset finished processing (READY) and rejects if it
 * failed. `onUpdate` receives progress while it is being processed.
 */
export async function waitForAsset(
  assetId: string,
  onUpdate?: (asset: media.Asset) => void,
  signal?: AbortSignal,
): Promise<media.Asset> {
  return uploads.waitForAsset(mediaClient, assetId, onUpdate, signal);
}

/** Upload and wait until the asset is usable: the common case for images. */
export async function uploadAndProcess(
  file: Blob & { name?: string },
  options: UploadOptions,
): Promise<media.Asset> {
  return uploads.processed(mediaClient, await uploadFile(file, options), options.signal);
}
