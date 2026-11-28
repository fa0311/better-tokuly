import type { Client } from "@connectrpc/connect";
import { ConnectError, media } from "@tokuly/api";
import { AssetFailedError, TransferError } from "@tokuly/core/errors";
import { type Account, ensureActive } from "#/features/session/account";
import { SERVER_URL } from "#/lib/config";
import { deleteFile, putFile, slicePart } from "#/platform/transfer";
import { learnLimit } from "./limits";

/**
 * Where one file's upload stands. Kept with the post it belongs to, so an
 * upload goes on from here after the app went to the background, lost the
 * network or was closed: the bytes of a single PUT go again, a multipart
 * upload asks the server which parts it already holds.
 */
export interface UploadState {
  /** The copy of the file kept on the device (`platform/transfer.ts`). */
  uri: string;
  name: string;
  contentType: string;
  size: number;
  uploadId?: string;
  assetId?: string;
  /** A single-request upload: where the bytes go, and the headers its signature covers. */
  putUrl?: string;
  putHeaders?: Record<string, string>;
  /** A multipart upload: every part is `partSize` bytes but the last. */
  partSize?: number;
  partCount?: number;
  /** The single PUT went through. */
  sent?: boolean;
  /** CompleteUpload went through: the asset exists and is (being) processed. */
  completed?: boolean;
}

export interface UploadOptions {
  /**
   * `ready`: resolves once the asset is processed and READY (what a story
   * attaches). `stored`: once the upload is complete and processing queued
   * (a short takes it then and goes live by itself when it is ready).
   */
  until: "ready" | "stored";
  /** Called with every change worth keeping (an upload started, the bytes are in, completed). */
  save(state: UploadState): void;
  /** 0..1 as bytes reach the store. */
  onProgress(fraction: number): void;
  signal: AbortSignal;
}

const absolute = (url: string) => (url.startsWith("/") ? `${SERVER_URL}${url}` : url);

/** The upload was discarded by the server (it expired, or never finished in time): it starts over once. */
function isGone(error: unknown): boolean {
  return (
    (error instanceof ConnectError &&
      ["UPLOAD_NOT_FOUND", "UPLOAD_NOT_ACTIVE"].includes(
        error.metadata.get("x-tokuly-error") ?? "",
      )) ||
    // A presigned URL that expired meanwhile.
    (error instanceof TransferError && error.status === 403)
  );
}

/**
 * Uploads one file as `account` and resolves once its asset is READY (or
 * stored, with `until: "stored"`); rejects with `AssetFailedError` when processing refused it (the server's
 * reason, e.g. MEDIA_TOO_LONG), with `AccountChanged` as soon as `account`
 * is no longer the active one, and with the network's error otherwise.
 * Nothing is ever sent as another account: every request is made with
 * `account`'s own API, and only while it is the active one.
 */
export async function uploadFile(
  account: Account,
  purpose: media.AssetPurpose,
  start: UploadState,
  options: UploadOptions,
): Promise<media.Asset> {
  try {
    return await upload(account, purpose, start, options);
  } catch (error) {
    if (!isGone(error)) throw error;
    const fresh: UploadState = {
      uri: start.uri,
      name: start.name,
      contentType: start.contentType,
      size: start.size,
    };
    options.save(fresh);
    return upload(account, purpose, fresh, options);
  }
}

async function upload(
  account: Account,
  purpose: media.AssetPurpose,
  start: UploadState,
  { until, save, onProgress, signal }: UploadOptions,
): Promise<media.Asset> {
  const client = account.api.service(media.MediaService);
  /** Every request: only while the account is the active one, and checked again once it answered. */
  const call = async <T>(request: () => Promise<T>): Promise<T> => {
    signal.throwIfAborted();
    ensureActive(account);
    const response = await request();
    ensureActive(account);
    return response;
  };
  let state = start;
  const keep = (change: Partial<UploadState>) => {
    state = { ...state, ...change };
    save(state);
  };

  if (!state.uploadId) {
    const created = await call(() =>
      client.createUpload(
        {
          purpose,
          fileName: state.name,
          contentType: state.contentType,
          sizeBytes: BigInt(state.size),
        },
        { signal },
      ),
    );
    learnLimit(purpose, created.maxDurationSeconds);
    keep({
      uploadId: created.uploadId,
      assetId: created.assetId,
      putUrl: created.putUrl || undefined,
      putHeaders: created.putUrl ? created.putHeaders : undefined,
      partSize: Number(created.partSizeBytes),
      partCount: created.partCount,
    });
  }
  const uploadId = state.uploadId!;

  let asset: media.Asset | undefined;
  if (!state.completed) {
    if (state.putUrl) {
      if (!state.sent) {
        await call(() =>
          putFile(
            absolute(state.putUrl!),
            state.uri,
            state.putHeaders ?? {},
            (sent) => onProgress(sent / Math.max(1, state.size)),
            signal,
          ),
        );
        keep({ sent: true });
      }
    } else {
      await sendParts(account, client, state, onProgress, signal);
    }
    onProgress(1);
    asset = (await call(() => client.completeUpload({ uploadId }, { signal }))).asset;
    keep({ completed: true });
  } else {
    asset = (await call(() => client.getAsset({ assetId: state.assetId! }, { signal }))).asset;
  }
  if (asset?.status === media.AssetStatus.FAILED)
    throw new AssetFailedError(asset.error || "PROCESSING_FAILED", asset.maxDurationSeconds);
  if (asset && (until === "stored" || asset.status === media.AssetStatus.READY)) return asset;

  ensureActive(account);
  for await (const update of client.watchAsset({ assetId: state.assetId! }, { signal })) {
    ensureActive(account);
    const current = update.asset;
    if (current?.status === media.AssetStatus.READY) return current;
    if (current?.status === media.AssetStatus.FAILED)
      throw new AssetFailedError(current.error || "PROCESSING_FAILED", current.maxDurationSeconds);
  }
  throw new AssetFailedError("PROCESSING_FAILED");
}

/** The parts the store does not hold yet, one after the other (one part's bytes in memory at a time). */
async function sendParts(
  account: Account,
  client: Client<typeof media.MediaService>,
  state: UploadState,
  onProgress: (fraction: number) => void,
  signal: AbortSignal,
): Promise<void> {
  const partSize = state.partSize ?? 0;
  const partCount = state.partCount ?? 0;
  const length = (part: number) => Math.min(partSize, state.size - (part - 1) * partSize);
  const ask = async (partNumbers: number[]) => {
    signal.throwIfAborted();
    ensureActive(account);
    const response = await client.getUploadParts(
      { uploadId: state.uploadId!, partNumbers },
      { signal },
    );
    ensureActive(account);
    return response;
  };
  const held = new Set((await ask([])).completedPartNumbers);
  let sent = [...held].reduce((sum, part) => sum + length(part), 0);
  for (let part = 1; part <= partCount; part++) {
    if (held.has(part)) continue;
    const url = (await ask([part])).parts[0]?.putUrl;
    if (!url) throw new Error(`no upload URL for part ${part}`);
    const partUri = await slicePart(state.uri, (part - 1) * partSize, length(part));
    try {
      ensureActive(account);
      await putFile(
        absolute(url),
        partUri,
        {},
        (bytes) => onProgress((sent + bytes) / Math.max(1, state.size)),
        signal,
      );
    } finally {
      deleteFile(partUri);
    }
    ensureActive(account);
    sent += length(part);
  }
}
