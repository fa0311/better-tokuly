import { common, create, video } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { validateWatchSearch } from "./search";
import {
  audienceKey,
  blockKind,
  chapterAt,
  descriptionParts,
  nextInQueue,
  playlistTitle,
  startPosition,
  withRating,
} from "./watch";

const { Rating } = video;

describe("the watch page's rules", () => {
  it("reads a watch link's search", () => {
    expect(validateWatchSearch({ t: 90, list: "WL", comment: "" })).toEqual({
      t: "90",
      list: "WL",
    });
    expect(validateWatchSearch({ t: {}, other: "x" })).toEqual({});
  });

  it("starts at the link's time, else where the viewer left off", () => {
    const summary = { durationSeconds: 600, resumePositionSeconds: 120, watched: false };
    expect(startPosition("1:30", summary)).toBe(90);
    expect(startPosition("2h", summary)).toBe(599);
    expect(startPosition(undefined, summary)).toBe(120);
    expect(startPosition(undefined, { ...summary, watched: true })).toBe(0);
    expect(startPosition(undefined, { ...summary, resumePositionSeconds: 2 })).toBe(0);
    expect(startPosition(undefined, { ...summary, resumePositionSeconds: 597 })).toBe(0);
  });

  it("finds the chapter playing", () => {
    const chapters = [0, 60, 120].map((startSeconds) =>
      create(video.ChapterSchema, { startSeconds, title: `at ${startSeconds}` }),
    );
    expect(chapterAt(chapters, 0)?.startSeconds).toBe(0);
    expect(chapterAt(chapters, 59.9)?.startSeconds).toBe(0);
    expect(chapterAt(chapters, 60)?.startSeconds).toBe(60);
    expect(chapterAt(chapters, 9999)?.startSeconds).toBe(120);
    expect(chapterAt([], 10)).toBeUndefined();
  });

  it("cuts a description into text, links and times", () => {
    expect(descriptionParts("見どころ 1:23 はこちら https://example.com/a.。")).toEqual([
      { kind: "text", text: "見どころ " },
      { kind: "time", text: "1:23", seconds: 83 },
      { kind: "text", text: " はこちら " },
      { kind: "link", text: "https://example.com/a" },
      { kind: "text", text: ".。" },
    ]);
    expect(descriptionParts("9:99 is no time")).toEqual([
      { kind: "text", text: "9:99 is no time" },
    ]);
  });

  it("names who may see a video, private when unknown", () => {
    expect(audienceKey(common.Visibility.PUBLIC, false)).toBeUndefined();
    expect(audienceKey(common.Visibility.FRIENDS, true)).toBe("watch.audience.friendsOrLink");
    expect(audienceKey(common.Visibility.CIRCLE, false)).toBe("watch.audience.circle");
    expect(audienceKey(99 as common.Visibility, false)).toBe("watch.audience.private");
  });

  it("says why there is no player", () => {
    const summary = create(video.VideoSummarySchema, { processing: video.ProcessingState.FAILED });
    expect(blockKind({ playbackBlock: video.PlaybackBlock.AGE_RESTRICTED, summary })).toBe("age");
    expect(blockKind({ playbackBlock: video.PlaybackBlock.NOT_READY, summary })).toBe("failed");
    expect(blockKind({ playbackBlock: video.PlaybackBlock.NOT_READY, summary: undefined })).toBe(
      "processing",
    );
  });

  it("moves the like count with the thumb", () => {
    const item = create(video.VideoSchema, {
      summary: { likeCount: 10n },
      viewer: { rating: Rating.NONE },
    });
    const liked = withRating(item, Rating.LIKE);
    expect(liked.summary?.likeCount).toBe(11n);
    expect(liked.viewer?.rating).toBe(Rating.LIKE);
    expect(withRating(liked, Rating.DISLIKE).summary?.likeCount).toBe(10n);
    expect(withRating(item, Rating.DISLIKE).summary?.likeCount).toBe(10n);
  });

  it("names the built-in playlists and finds what comes next", () => {
    const t = (key: string) => `<${key}>`;
    expect(playlistTitle({ kind: video.PlaylistKind.WATCH_LATER, title: "" }, t)).toBe(
      "<playlist.watchLater>",
    );
    expect(playlistTitle({ kind: video.PlaylistKind.CUSTOM, title: "mix" }, t)).toBe("mix");
    const items = ["a", "b"].map((id) => create(video.VideoSummarySchema, { id }));
    expect(nextInQueue(items, "a")?.id).toBe("b");
    expect(nextInQueue(items, "b")).toBeUndefined();
    expect(nextInQueue(items, "z")).toBeUndefined();
  });
});
