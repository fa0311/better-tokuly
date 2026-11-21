/**
 * Just enough of the ZIP format to open a dotLottie file: the list of
 * entries from the central directory, and their bytes (stored or deflated;
 * inflating is done by the platform's `DecompressionStream`).
 */

export interface ZipEntry {
  name: string;
  /** 0 = stored, 8 = deflate. */
  method: number;
  compressedSize: number;
  size: number;
  /** Position of the entry's local header. */
  offset: number;
}

const END_OF_CENTRAL_DIRECTORY = 0x06054b50;
const CENTRAL_ENTRY = 0x02014b50;
const LOCAL_ENTRY = 0x04034b50;

/** The files in a ZIP archive. Throws when the bytes are not a ZIP. */
export function listZipEntries(bytes: Uint8Array): ZipEntry[] {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  // The end record is at the very end, followed only by an optional comment (up to 64 KiB).
  let end = -1;
  for (let position = bytes.length - 22; position >= Math.max(0, bytes.length - 22 - 0xffff); position--) {
    if (view.getUint32(position, true) === END_OF_CENTRAL_DIRECTORY) {
      end = position;
      break;
    }
  }
  if (end < 0) throw new Error("not a ZIP archive");

  const count = view.getUint16(end + 10, true);
  let position = view.getUint32(end + 16, true);
  const decoder = new TextDecoder();
  const entries: ZipEntry[] = [];
  for (let index = 0; index < count; index++) {
    if (position + 46 > bytes.length || view.getUint32(position, true) !== CENTRAL_ENTRY) throw new Error("damaged ZIP directory");
    const nameLength = view.getUint16(position + 28, true);
    const extraLength = view.getUint16(position + 30, true);
    const commentLength = view.getUint16(position + 32, true);
    entries.push({
      name: decoder.decode(bytes.subarray(position + 46, position + 46 + nameLength)),
      method: view.getUint16(position + 10, true),
      compressedSize: view.getUint32(position + 20, true),
      size: view.getUint32(position + 24, true),
      offset: view.getUint32(position + 42, true),
    });
    position += 46 + nameLength + extraLength + commentLength;
  }
  return entries;
}

/** The uncompressed bytes of one entry. */
export async function readZipEntry(bytes: Uint8Array, entry: ZipEntry): Promise<Uint8Array> {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (entry.offset + 30 > bytes.length || view.getUint32(entry.offset, true) !== LOCAL_ENTRY) throw new Error("damaged ZIP entry");
  const start = entry.offset + 30 + view.getUint16(entry.offset + 26, true) + view.getUint16(entry.offset + 28, true);
  const compressed = bytes.subarray(start, start + entry.compressedSize);
  if (entry.method === 0) return compressed;
  if (entry.method !== 8) throw new Error(`unsupported ZIP compression (${entry.method})`);
  const inflated = new Blob([new Uint8Array(compressed)]).stream().pipeThrough(new DecompressionStream("deflate-raw"));
  return new Uint8Array(await new Response(inflated).arrayBuffer());
}
