/**
 * Stream framing for the WebTransport path: every frame on the reliable
 * stream is prefixed with its length as a 4-byte big-endian integer.
 */

/** Largest frame we are willing to buffer (protects against a corrupt length prefix). */
export const MAX_FRAME_BYTES = 4 * 1024 * 1024;

export function encodeFrame(payload: Uint8Array): Uint8Array {
  const out = new Uint8Array(4 + payload.length);
  new DataView(out.buffer).setUint32(0, payload.length, false);
  out.set(payload, 4);
  return out;
}

/** Accumulates stream chunks and yields complete frames. */
export class FrameDecoder {
  private buffer: Uint8Array = new Uint8Array(0);

  /** Feeds a chunk and returns every frame completed by it. */
  push(chunk: Uint8Array): Uint8Array[] {
    if (this.buffer.length === 0) {
      this.buffer = chunk;
    } else {
      const merged = new Uint8Array(this.buffer.length + chunk.length);
      merged.set(this.buffer, 0);
      merged.set(chunk, this.buffer.length);
      this.buffer = merged;
    }
    const frames: Uint8Array[] = [];
    let offset = 0;
    while (this.buffer.length - offset >= 4) {
      const view = new DataView(this.buffer.buffer, this.buffer.byteOffset + offset, 4);
      const length = view.getUint32(0, false);
      if (length > MAX_FRAME_BYTES) throw new Error(`realtime frame too large: ${length} bytes`);
      if (this.buffer.length - offset - 4 < length) break;
      frames.push(this.buffer.slice(offset + 4, offset + 4 + length));
      offset += 4 + length;
    }
    this.buffer = offset === 0 ? this.buffer : this.buffer.slice(offset);
    return frames;
  }
}
