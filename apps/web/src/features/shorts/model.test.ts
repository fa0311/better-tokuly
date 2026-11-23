import { shorts } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import {
  captionParts,
  clock,
  completeToken,
  contextOfSearch,
  playbackSource,
  searchOfContext,
  slideLoad,
  tokenAtCaret,
  validateShortSearch,
} from "./model";

describe("captionParts", () => {
  it("links the hashtags and mentions the server read out", () => {
    expect(
      captionParts("文化祭だよ #文化祭 @Miki みてね", ["文化祭"], [{ handle: "miki" }]),
    ).toEqual([
      { kind: "text", text: "文化祭だよ " },
      { kind: "hashtag", text: "#文化祭", tag: "文化祭" },
      { kind: "text", text: " " },
      { kind: "mention", text: "@Miki", handle: "miki" },
      { kind: "text", text: " みてね" },
    ]);
  });

  it("leaves names that name nobody, and unknown tags, as text", () => {
    expect(captionParts("@nobody #x は?", [], [])).toEqual([
      { kind: "text", text: "@nobody #x は?" },
    ]);
  });

  it("keeps a caption without tokens whole", () => {
    expect(captionParts("ただのキャプション", [], [])).toEqual([
      { kind: "text", text: "ただのキャプション" },
    ]);
    expect(captionParts("", [], [])).toEqual([]);
  });
});

describe("tokenAtCaret", () => {
  it("finds the hashtag being typed", () => {
    expect(tokenAtCaret("ねこ #ぶん", 6)).toEqual({ kind: "hashtag", query: "ぶん", start: 3 });
    expect(tokenAtCaret("#", 1)).toEqual({ kind: "hashtag", query: "", start: 0 });
  });

  it("finds the mention being typed", () => {
    expect(tokenAtCaret("hi @mi", 6)).toEqual({ kind: "mention", query: "mi", start: 3 });
  });

  it("ignores a sign in the middle of a word, and finished tokens", () => {
    expect(tokenAtCaret("mail@example", 12)).toBeNull();
    expect(tokenAtCaret("#ねこ ", 4)).toBeNull();
    expect(tokenAtCaret("@みき", 3)).toBeNull();
  });
});

describe("completeToken", () => {
  it("replaces the token and puts the caret after it", () => {
    expect(completeToken("ねこ #ぶん だよ", 3, 6, "#文化祭")).toEqual({
      text: "ねこ #文化祭 だよ",
      caret: 8,
    });
  });
});

describe("the address of a short", () => {
  it("keeps the list a short was opened from", () => {
    expect(validateShortSearch({ list: "tag", key: "ねこ" })).toEqual({ list: "tag", key: "ねこ" });
    expect(validateShortSearch({ list: "saved", key: "x" })).toEqual({ list: "saved" });
    expect(validateShortSearch({ list: "tag" })).toEqual({});
    expect(validateShortSearch({ list: "nope", via: "search" })).toEqual({ via: "search" });
  });

  it("round-trips lists through the address", () => {
    for (const search of [
      { list: "channel", key: "tokuly" },
      { list: "tag", key: "ねこ" },
      { list: "sound", key: "abc" },
      { list: "remixes", key: "abc" },
      { list: "saved" },
      { list: "liked" },
    ] as const) {
      expect(searchOfContext(contextOfSearch(search))).toEqual(search);
    }
  });

  it("names the place a short was watched for the report", () => {
    const { PlaybackSource } = shorts;
    expect(playbackSource({ kind: "forYou" })).toBe(PlaybackSource.FOR_YOU);
    expect(playbackSource({ kind: "hashtag", key: "ねこ" })).toBe(PlaybackSource.HASHTAG);
    expect(playbackSource({ kind: "channel", key: "tokuly" })).toBe(PlaybackSource.PROFILE);
    expect(playbackSource({ kind: "link" })).toBe(PlaybackSource.SHARE_LINK);
    expect(playbackSource({ kind: "link", via: "notification" })).toBe(PlaybackSource.NOTIFICATION);
    expect(playbackSource({ kind: "saved" })).toBe(PlaybackSource.OTHER);
  });
});

describe("slideLoad", () => {
  it("gives a video to the current short and the next one only", () => {
    const loads = [0, 1, 2, 3, 4, 5, 6].map((index) => slideLoad(index, 3));
    expect(loads).toEqual(["empty", "empty", "cover", "playing", "preloading", "cover", "empty"]);
    expect(loads.filter((load) => load === "playing" || load === "preloading")).toHaveLength(2);
  });
});

describe("clock", () => {
  it("formats seconds", () => {
    expect(clock(0)).toBe("0:00");
    expect(clock(83.4)).toBe("1:23");
    expect(clock(180)).toBe("3:00");
  });
});
