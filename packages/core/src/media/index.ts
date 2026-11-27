/**
 * Uploads as every app runs them: the asset is created, the bytes go straight
 * to object storage (one request, or parts side by side for a large file),
 * the upload is completed — or aborted, never left half-finished — and the
 * asset's processing is waited for. Moving the bytes is the app's: it hands
 * over its own `put` (XMLHttpRequest, for its progress events).
 */
export {
  type Bytes,
  type PutBytes,
  processed,
  type UploadOptions,
  type UploadRequest,
  uploadBytes,
  waitForAsset,
} from "./upload";
