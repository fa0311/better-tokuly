import { screenFor } from "#/seams/links";

describe("video links", () => {
  it("open the app's own video screens", () => {
    expect(screenFor("/watch")).toBe("/watch");
    expect(screenFor("/watch/abc123")).toBe("/watch/abc123");
    expect(screenFor("/watch/history")).toBe("/watch/history");
    expect(screenFor("/watch/playlists")).toBe("/watch/library");
    expect(screenFor("/c/natsu")).toBe("/c/natsu");
    expect(screenFor("/c/natsu/videos")).toBe("/c/natsu");
    expect(screenFor("/playlist/WL")).toBe("/playlist/WL");
  });

  it("leave the studio and watch parties on the web", () => {
    expect(screenFor("/studio/videos/abc123")).toBe(
      `/web?path=${encodeURIComponent("/studio/videos/abc123")}`,
    );
    expect(screenFor("/party/p1")).toBe(`/web?path=${encodeURIComponent("/party/p1")}`);
  });
});
