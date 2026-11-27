import { describe, expect, it } from "vitest";
import { classify, needsConversion } from "./attachments";
import { extensionOf } from "./recorder";

describe("files", () => {
  it("are sent as what they are", () => {
    expect(classify("image/jpeg")).toBe("image");
    expect(classify("image/gif")).toBe("image");
    expect(classify("video/mp4")).toBe("video");
    expect(classify("audio/webm")).toBe("audio");
    // Photos the server cannot decode are still photos: converted before upload, never sent as files.
    for (const type of ["image/heic", "image/heif", "image/avif", "image/bmp"]) {
      expect(classify(type)).toBe("image");
      expect(needsConversion(type)).toBe(true);
    }
    expect(needsConversion("image/jpeg")).toBe(false);
    expect(needsConversion("image/webp")).toBe(false);
    // Pictures that are documents rather than photos go as files.
    expect(classify("image/svg+xml")).toBe("file");
    expect(classify("application/pdf")).toBe("file");
    expect(classify("")).toBe("file");
  });

  it("get the extension of the container that was recorded", () => {
    expect(extensionOf("audio/webm;codecs=opus")).toBe("webm");
    expect(extensionOf("audio/mp4")).toBe("m4a");
    expect(extensionOf("video/mp4")).toBe("mp4");
    expect(extensionOf("audio/ogg;codecs=opus")).toBe("ogg");
  });
});
