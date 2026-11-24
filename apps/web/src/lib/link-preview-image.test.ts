import { describe, expect, it } from "vitest";
import { linkPreviewImage } from "./link-preview-image";

const ORIGIN = "https://tokuly.example";

describe("linkPreviewImage", () => {
  it("loads proxied pictures from the app's own origin", () => {
    expect(
      linkPreviewImage(
        "https://tokuly.example/api/media/link-image/image/sig/aHR0cHM6Ly9h",
        ORIGIN,
      ),
    ).toBe("https://tokuly.example/api/media/link-image/image/sig/aHR0cHM6Ly9h");
    // The server's public URL may differ from the origin the app runs on (development).
    expect(
      linkPreviewImage("http://localhost:8080/api/media/link-image/icon/sig/aHR0cHM6Ly9h", ORIGIN),
    ).toBe("https://tokuly.example/api/media/link-image/icon/sig/aHR0cHM6Ly9h");
  });

  it("never loads a picture from the linked site", () => {
    expect(linkPreviewImage("https://news.example.com/cover.jpg", ORIGIN)).toBeUndefined();
    expect(linkPreviewImage("https://i.ytimg.com/vi/x/hqdefault.jpg", ORIGIN)).toBeUndefined();
    expect(
      linkPreviewImage("https://evil.example/api/media/link-image/x?y=1", ORIGIN),
    ).toBeUndefined();
    expect(linkPreviewImage("https://tokuly.example/api/media/other/x", ORIGIN)).toBeUndefined();
    expect(linkPreviewImage("", ORIGIN)).toBeUndefined();
    expect(linkPreviewImage(undefined, ORIGIN)).toBeUndefined();
  });
});
