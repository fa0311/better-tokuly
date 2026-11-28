import { LocalFileError, NetworkError, TransferError } from "@tokuly/core/errors";
import { Directory, File, Paths } from "expo-file-system";

/**
 * Moving the bytes of an upload: the device side of the media flow. Files to
 * upload are kept in the app's own documents folder (a library pick or a
 * recording may live somewhere the OS clears), so an upload can resume after
 * the app was closed. PUTs go through the OS's own transfer: on iOS a
 * background URL session, which goes on while the app is suspended.
 */

const ROOT = new Directory(Paths.document, "uploads");

function folder(postId: string): Directory {
  if (!ROOT.exists) ROOT.create({ idempotent: true });
  const directory = new Directory(ROOT, postId);
  if (!directory.exists) directory.create({ idempotent: true });
  return directory;
}

/**
 * Copies a picked or recorded file into the post's folder; resolves to the
 * kept copy's URI and size, or rejects with a `LocalFileError` when the file
 * cannot be copied (one the OS took back, or a folder it refused).
 */
export async function keepFile(
  postId: string,
  uri: string,
  name: string,
): Promise<{ uri: string; size: number }> {
  try {
    const kept = new File(folder(postId), name);
    if (kept.exists) kept.delete();
    await new File(uri).copy(kept);
    return { uri: kept.uri, size: kept.size };
  } catch (error) {
    throw new LocalFileError(`cannot keep ${name}`, { cause: error });
  }
}

/** Throws away everything kept for a post (posted, or given up). */
export function forgetFiles(postId: string): void {
  const directory = new Directory(ROOT, postId);
  if (directory.exists) directory.delete();
}

/** The posts whose files are on the device. */
export function keptPosts(): string[] {
  if (!ROOT.exists) return [];
  return ROOT.list()
    .filter((entry) => entry instanceof Directory)
    .map((entry) => entry.name);
}

const QUEUE = "queue.json";

/** The posts on their way, as last written (`[]` when there are none or the file is unreadable). */
export async function readQueue<T>(): Promise<T[]> {
  const file = new File(ROOT, QUEUE);
  if (!file.exists) return [];
  try {
    return JSON.parse(await file.text()) as T[];
  } catch {
    return [];
  }
}

export function writeQueue<T>(posts: T[]): void {
  if (!ROOT.exists) ROOT.create({ idempotent: true });
  new File(ROOT, QUEUE).writeSync(JSON.stringify(posts));
}

/** Writes bytes `[offset, offset + length)` of a file to a file of their own, for one part of a multipart upload. */
export async function slicePart(uri: string, offset: number, length: number): Promise<string> {
  const source = new File(uri);
  const part = new File(Paths.cache, `part-${source.name}-${offset}`);
  const handle = source.open();
  try {
    handle.offset = offset;
    const bytes = await handle.readBytes(length);
    if (part.exists) part.delete();
    part.create();
    await part.write(bytes);
  } finally {
    handle.close();
  }
  return part.uri;
}

export function deleteFile(uri: string): void {
  const file = new File(uri);
  if (file.exists) file.delete();
}

/**
 * PUTs a file as the request body. Resolves once the store took it; rejects
 * with a `TransferError` for a refusal, with an `AbortError` when `signal`
 * fired, and with a `NetworkError` otherwise.
 */
export async function putFile(
  url: string,
  uri: string,
  headers: Record<string, string>,
  onProgress: (sent: number) => void,
  signal: AbortSignal,
): Promise<void> {
  const task = new File(uri).createUploadTask(url, {
    httpMethod: "PUT",
    headers,
    sessionType: "background",
    onProgress: ({ bytesSent }) => onProgress(bytesSent),
    signal,
  });
  let result: Awaited<ReturnType<typeof task.uploadAsync>>;
  try {
    result = await task.uploadAsync();
  } catch (error) {
    if (signal.aborted) throw error;
    throw new NetworkError("upload failed (network)", { cause: error });
  } finally {
    task.release();
  }
  if (result.status < 200 || result.status >= 300) throw new TransferError(result.status);
}
