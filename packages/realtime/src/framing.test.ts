import { describe, expect, it } from "vitest";
import { encodeFrame, FrameDecoder, MAX_FRAME_BYTES } from "./framing.js";

const bytes = (...values: number[]) => new Uint8Array(values);

describe("stream framing", () => {
  it("prefixes frames with a big-endian length", () => {
    expect([...encodeFrame(bytes(7, 8, 9))]).toEqual([0, 0, 0, 3, 7, 8, 9]);
    expect([...encodeFrame(bytes())]).toEqual([0, 0, 0, 0]);
  });

  it("reassembles frames split across arbitrary chunk boundaries", () => {
    const frames = [bytes(1), bytes(), bytes(2, 3, 4), new Uint8Array(1000).fill(5)];
    const wire = frames.map(encodeFrame).reduce((all, part) => {
      const merged = new Uint8Array(all.length + part.length);
      merged.set(all);
      merged.set(part, all.length);
      return merged;
    }, new Uint8Array(0));
    for (const chunkSize of [1, 2, 3, 5, 64, wire.length]) {
      const decoder = new FrameDecoder();
      const decoded: Uint8Array[] = [];
      for (let offset = 0; offset < wire.length; offset += chunkSize) {
        decoded.push(...decoder.push(wire.slice(offset, offset + chunkSize)));
      }
      expect(decoded.map((frame) => [...frame])).toEqual(frames.map((frame) => [...frame]));
    }
  });

  it("returns several frames delivered in one chunk and keeps the remainder", () => {
    const decoder = new FrameDecoder();
    const first = encodeFrame(bytes(1, 2));
    const second = encodeFrame(bytes(3));
    const chunk = new Uint8Array([...first, ...second, 0, 0]);
    expect(decoder.push(chunk).map((frame) => [...frame])).toEqual([[1, 2], [3]]);
    expect(decoder.push(bytes(0, 1, 9)).map((frame) => [...frame])).toEqual([[9]]);
  });

  it("rejects an absurd length prefix instead of buffering forever", () => {
    const decoder = new FrameDecoder();
    const header = new Uint8Array(4);
    new DataView(header.buffer).setUint32(0, MAX_FRAME_BYTES + 1, false);
    expect(() => decoder.push(header)).toThrow(/too large/);
  });
});
