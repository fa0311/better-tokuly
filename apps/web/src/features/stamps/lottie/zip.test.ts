import { deflateRawSync } from "node:zlib";
import { describe, expect, it } from "vitest";
import { animationEntry } from "./dotlottie";
import { listZipEntries, readZipEntry } from "./zip";

/** A ZIP archive built by hand: each file stored as it is, or deflated. */
function zip(files: { name: string; text: string; deflate?: boolean }[], comment = ""): Uint8Array {
  const encoder = new TextEncoder();
  const parts: Uint8Array[] = [];
  const directory: Uint8Array[] = [];
  let offset = 0;
  for (const file of files) {
    const name = encoder.encode(file.name);
    const data = encoder.encode(file.text);
    const stored = file.deflate ? new Uint8Array(deflateRawSync(data)) : data;
    const local = new DataView(new ArrayBuffer(30));
    local.setUint32(0, 0x04034b50, true);
    local.setUint16(8, file.deflate ? 8 : 0, true);
    local.setUint32(18, stored.length, true);
    local.setUint32(22, data.length, true);
    local.setUint16(26, name.length, true);
    const central = new DataView(new ArrayBuffer(46));
    central.setUint32(0, 0x02014b50, true);
    central.setUint16(10, file.deflate ? 8 : 0, true);
    central.setUint32(20, stored.length, true);
    central.setUint32(24, data.length, true);
    central.setUint16(28, name.length, true);
    central.setUint32(42, offset, true);
    parts.push(new Uint8Array(local.buffer), name, stored);
    directory.push(new Uint8Array(central.buffer), name);
    offset += 30 + name.length + stored.length;
  }
  const note = encoder.encode(comment);
  const end = new DataView(new ArrayBuffer(22));
  end.setUint32(0, 0x06054b50, true);
  end.setUint16(8, files.length, true);
  end.setUint16(10, files.length, true);
  end.setUint32(12, directory.reduce((sum, part) => sum + part.length, 0), true);
  end.setUint32(16, offset, true);
  end.setUint16(20, note.length, true);
  const all = [...parts, ...directory, new Uint8Array(end.buffer), note];
  const out = new Uint8Array(all.reduce((sum, part) => sum + part.length, 0));
  let position = 0;
  for (const part of all) {
    out.set(part, position);
    position += part.length;
  }
  return out;
}

const text = async (bytes: Uint8Array, name: string) => {
  const entry = listZipEntries(bytes).find((candidate) => candidate.name === name);
  if (!entry) throw new Error(`no entry ${name}`);
  return new TextDecoder().decode(await readZipEntry(bytes, entry));
};

describe("zip reader", () => {
  it("lists the entries and reads stored and deflated files", async () => {
    const long = JSON.stringify({ layers: Array.from({ length: 50 }, (_, index) => ({ ind: index, nm: "まる" })) });
    const archive = zip([
      { name: "manifest.json", text: '{"animations":[{"id":"a"}]}' },
      { name: "animations/a.json", text: long, deflate: true },
    ]);

    expect(listZipEntries(archive).map((entry) => [entry.name, entry.method])).toEqual([
      ["manifest.json", 0],
      ["animations/a.json", 8],
    ]);
    expect(await text(archive, "manifest.json")).toBe('{"animations":[{"id":"a"}]}');
    expect(await text(archive, "animations/a.json")).toBe(long);
  });

  it("finds the directory behind an archive comment", async () => {
    const archive = zip([{ name: "a.txt", text: "hello" }], "made by a test");
    expect(await text(archive, "a.txt")).toBe("hello");
  });

  it("refuses bytes that are not a ZIP archive", () => {
    expect(() => listZipEntries(new TextEncoder().encode("this is definitely not a zip archive"))).toThrow("not a ZIP archive");
  });
});

describe("the animation inside a dotLottie file", () => {
  const entries = listZipEntries(
    zip([
      { name: "manifest.json", text: "{}" },
      { name: "animations/wave.json", text: "{}" },
      { name: "a/jump.json", text: "{}" },
    ]),
  );

  it("is the manifest's first animation, in either folder layout", () => {
    expect(animationEntry(entries, { animations: [{ id: "wave" }] })?.name).toBe("animations/wave.json");
    expect(animationEntry(entries, { animations: [{ id: "jump" }] })?.name).toBe("a/jump.json");
  });

  it("falls back to the first JSON file that is not the manifest", () => {
    expect(animationEntry(entries, undefined)?.name).toBe("animations/wave.json");
    expect(animationEntry(entries, { animations: [{ id: "missing" }] })?.name).toBe("animations/wave.json");
  });
});
