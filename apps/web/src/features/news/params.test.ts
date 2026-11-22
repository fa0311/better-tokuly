import { feed } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { listRequest, parseNewsParams, toggledFeedback } from "./params";

describe("the address of the news list", () => {
  it("reads order, tag, source and language", () => {
    expect(parseNewsParams({ order: "latest", tag: " Tech ", source: "s1", lang: "ja" })).toEqual({ order: "latest", tag: "tech", source: "s1", lang: "ja" });
  });

  it("is the recommended list without parameters, and ignores what it does not know", () => {
    expect(parseNewsParams({})).toEqual({ order: undefined, tag: undefined, source: undefined, lang: undefined });
    expect(parseNewsParams({ order: "oldest", tag: "  ", source: {} })).toEqual({ order: undefined, tag: undefined, source: undefined, lang: undefined });
  });

  it("keeps a tag the router parsed into a number", () => {
    expect(parseNewsParams({ tag: 2026 }).tag).toBe("2026");
  });

  it("becomes the API request of a page", () => {
    expect(listRequest({ order: "latest", tag: "tech", source: "s1", lang: "en" }, "c1", 20)).toEqual({ page: { cursor: "c1", limit: 20 }, order: feed.NewsOrder.LATEST, tag: "tech", sourceId: "s1", language: "en" });
    expect(listRequest({}, "", 20)).toMatchObject({ order: feed.NewsOrder.RECOMMENDED, tag: "", sourceId: "", language: "" });
  });
});

describe("feedback buttons", () => {
  it("choose an answer, and take it back when pressed again", () => {
    const { UNSPECIFIED, MORE, LESS } = feed.NewsFeedback;
    expect(toggledFeedback(UNSPECIFIED, MORE)).toBe(MORE);
    expect(toggledFeedback(MORE, MORE)).toBe(UNSPECIFIED);
    expect(toggledFeedback(MORE, LESS)).toBe(LESS);
  });
});
