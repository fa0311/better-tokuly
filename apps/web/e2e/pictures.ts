import zlib from "node:zlib";
import type { APIRequestContext } from "@playwright/test";
import { type Account, expect, rpc } from "./support";

// Pictures made for the tests, and their upload: custom emoji (emoji, engagement) need real images.

const MEDIA = "tokuly.media.v1.MediaService";

export interface TestFile {
  name: string;
  mimeType: string;
  buffer: Buffer;
}

export type Rgb = [number, number, number];

/** A PNG picture: a coloured disc on a transparent background. */
export function discPicture(name: string, color: Rgb = [230, 40, 60], size = 128): TestFile {
  const row = size * 4 + 1;
  const raw = Buffer.alloc(row * size);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      if (Math.hypot(x - size / 2, y - size / 2) < size * 0.42)
        raw.set([...color, 255], y * row + 1 + x * 4);
    }
  }
  const chunk = (type: string, data: Buffer) => {
    const body = Buffer.concat([Buffer.from(type), data]);
    const out = Buffer.alloc(body.length + 8);
    out.writeUInt32BE(data.length, 0);
    body.copy(out, 4);
    out.writeUInt32BE(zlib.crc32(body) >>> 0, body.length + 4);
    return out;
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(size, 0);
  header.writeUInt32BE(size, 4);
  // 8 bits per channel, RGBA.
  header.set([8, 6, 0, 0, 0], 8);
  const signature = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  return {
    name,
    mimeType: "image/png",
    buffer: Buffer.concat([
      signature,
      chunk("IHDR", header),
      chunk("IDAT", zlib.deflateSync(raw)),
      chunk("IEND", Buffer.alloc(0)),
    ]),
  };
}

/**
 * An animated GIF: two frames, a red square then a blue one. The pixels are
 * written as literal LZW codes with a "clear" in between, which every decoder
 * reads and no encoder is needed for.
 */
export function blinkingGif(name: string, size = 32): TestFile {
  const frame = (colorIndex: number) => {
    // 3-bit codes: 0-3 are the colours, 4 clears the table, 5 ends the picture.
    const codes: number[] = [];
    for (let pixel = 0; pixel < size * size; pixel++) {
      if (pixel % 2 === 0) codes.push(4);
      codes.push(colorIndex);
    }
    codes.push(5);
    const bytes: number[] = [];
    let bits = 0;
    let filled = 0;
    for (const code of codes) {
      bits |= code << filled;
      filled += 3;
      while (filled >= 8) {
        bytes.push(bits & 0xff);
        bits >>= 8;
        filled -= 8;
      }
    }
    if (filled > 0) bytes.push(bits & 0xff);
    const blocks: number[] = [];
    for (let at = 0; at < bytes.length; at += 255) {
      const part = bytes.slice(at, at + 255);
      blocks.push(part.length, ...part);
    }
    const descriptor = Buffer.alloc(10);
    descriptor.writeUInt8(0x2c, 0);
    descriptor.writeUInt16LE(size, 5);
    descriptor.writeUInt16LE(size, 7);
    // Graphic control: 0.4 s per frame.
    return Buffer.concat([
      Buffer.from([0x21, 0xf9, 4, 0, 40, 0, 0, 0]),
      descriptor,
      Buffer.from([2, ...blocks, 0]),
    ]);
  };
  const screen = Buffer.alloc(7);
  screen.writeUInt16LE(size, 0);
  screen.writeUInt16LE(size, 2);
  // A global table of four colours.
  screen.writeUInt8(0x91, 4);
  const colors = Buffer.from([255, 255, 255, 230, 40, 60, 60, 120, 240, 0, 0, 0]);
  const loop = Buffer.from([0x21, 0xff, 11, ...Buffer.from("NETSCAPE2.0"), 3, 1, 0, 0, 0]);
  return {
    name,
    mimeType: "image/gif",
    buffer: Buffer.concat([
      Buffer.from("GIF89a"),
      screen,
      colors,
      loop,
      frame(1),
      frame(2),
      Buffer.from([0x3b]),
    ]),
  };
}

/** Uploads a picture for an emoji (purpose STAMP) and waits until it is processed. Returns the asset ID. */
export async function uploadPicture(
  request: APIRequestContext,
  as: Account,
  file: TestFile,
): Promise<string> {
  const created = await rpc<{
    assetId: string;
    uploadId: string;
    putUrl: string;
    putHeaders?: Record<string, string>;
  }>(
    request,
    `${MEDIA}/CreateUpload`,
    {
      purpose: "ASSET_PURPOSE_STAMP",
      fileName: file.name,
      contentType: file.mimeType,
      sizeBytes: String(file.buffer.length),
    },
    as,
  );
  const put = await request.put(created.putUrl, {
    data: file.buffer,
    headers: created.putHeaders ?? {},
  });
  expect(put.ok(), `upload of ${file.name}`).toBeTruthy();
  await rpc(request, `${MEDIA}/CompleteUpload`, { uploadId: created.uploadId }, as);
  await expect(async () => {
    const { asset } = await rpc<{ asset: { status: string } }>(
      request,
      `${MEDIA}/GetAsset`,
      { assetId: created.assetId },
      as,
    );
    expect(asset.status).toBe("ASSET_STATUS_READY");
  }).toPass({ timeout: 30_000 });
  return created.assetId;
}
