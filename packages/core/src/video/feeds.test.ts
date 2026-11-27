import { create, video } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { feedKey, feedVideos } from "./feeds";
import { videoKeys } from "./keys";
import { CATEGORIES } from "./labels";
import { validateFeedSearch } from "./search";

const { VideoCategory } = video;

const card = (id: string, channelId: string, category = VideoCategory.GAMING) =>
  create(video.VideoSummarySchema, { id, category, channel: { id: channelId } });

describe("video feeds", () => {
  it("reads the feed and genre, falling back to home", () => {
    expect(validateFeedSearch({ feed: "trending", category: "1" })).toEqual({
      feed: "trending",
      category: VideoCategory.GAMING,
    });
    expect(validateFeedSearch({ feed: "home", category: "999" })).toEqual({});
    expect(validateFeedSearch({ feed: "nope" })).toEqual({});
  });

  it("takes every genre the chips offer", () => {
    for (const { value } of CATEGORIES) {
      expect(validateFeedSearch({ category: value })).toEqual({ category: value });
    }
    expect(validateFeedSearch({ category: VideoCategory.UNSPECIFIED })).toEqual({});
  });

  it("keys trending by genre only", () => {
    expect(feedKey("home", VideoCategory.MUSIC)).toEqual(videoKeys.home);
    expect(feedKey("trending", VideoCategory.MUSIC)).toEqual(
      videoKeys.trending(VideoCategory.MUSIC),
    );
  });

  it("shows each video once, of the genre, without a dismissed channel's others", () => {
    const pages = [
      { videos: [card("a", "c1"), card("b", "c2"), card("m", "c2", VideoCategory.MUSIC)] },
      { videos: [card("a", "c1"), card("c", "c2")] },
    ];
    expect(feedVideos(pages, "home", VideoCategory.UNSPECIFIED).map((v) => v.id)).toEqual([
      "a",
      "b",
      "m",
      "c",
    ]);
    expect(feedVideos(pages, "home", VideoCategory.GAMING).map((v) => v.id)).toEqual([
      "a",
      "b",
      "c",
    ]);
    // Trending is filtered by the server.
    expect(feedVideos(pages, "trending", VideoCategory.GAMING)).toHaveLength(4);
    const dismissed = { b: { kind: "channel" as const, channelId: "c2" } };
    expect(
      feedVideos(pages, "home", VideoCategory.UNSPECIFIED, dismissed).map((v) => v.id),
    ).toEqual(["a", "b"]);
  });
});
