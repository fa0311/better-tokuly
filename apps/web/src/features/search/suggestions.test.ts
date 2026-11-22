import { search } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { idleSuggestions, typedSuggestions } from "./suggestions";

const completion = (query: string, source: search.CompletionSource, recentSearchId = "") =>
  ({ query, source, recentSearchId }) as search.Completion;
const hit = (kind: search.SearchKind, id: string, title: string) =>
  ({ kind, id, title }) as search.SearchHit;
const recent = (id: string, query: string) => ({ id, query }) as search.RecentSearch;
const trend = (query: string, rank: number) => ({ query, rank }) as search.TrendingQuery;

describe("suggestions while typing", () => {
  it("lists completions, then hits, then the search for the typed text", () => {
    const list = typedSuggestions(" さく ", {
      completions: [
        completion("さくら", search.CompletionSource.RECENT, "r1"),
        completion("さくらんぼ", search.CompletionSource.POPULAR),
      ],
      hits: [
        hit(search.SearchKind.USER, "u1", "さくら"),
        hit(search.SearchKind.HASHTAG, "h1", "#さくら"),
      ],
    });
    expect(list.map((item) => item.group)).toEqual([
      "completions",
      "completions",
      "hits",
      "hits",
      "search",
    ]);
    expect(list[0]).toMatchObject({
      type: "query",
      query: "さくら",
      source: "recent",
      recentId: "r1",
    });
    expect(list[1]).toMatchObject({
      type: "query",
      query: "さくらんぼ",
      source: "popular",
      recentId: undefined,
    });
    expect(list.at(-1)).toMatchObject({ type: "search", query: "さく" });
    expect(new Set(list.map((item) => item.id)).size).toBe(list.length);
  });

  it("offers the search for the text before the API has answered", () => {
    expect(typedSuggestions("カフェ", undefined)).toEqual([
      { type: "search", group: "search", id: "search", query: "カフェ" },
    ]);
  });

  it("offers nothing for blank input", () => {
    expect(typedSuggestions("   ", { completions: [], hits: [] })).toEqual([]);
  });
});

describe("suggestions before typing", () => {
  it("shows recent searches, then trends that are not among them", () => {
    const list = idleSuggestions(
      [recent("r1", "カフェ"), recent("r2", "さくら")],
      [trend("カフェ", 1), trend("文化祭", 2), trend("テスト", 3)],
    );
    expect(
      list.map((item) => (item.type === "query" ? `${item.group}:${item.query}` : "")),
    ).toEqual(["recent:カフェ", "recent:さくら", "trending:文化祭", "trending:テスト"]);
    expect(list[0]).toMatchObject({ recentId: "r1" });
    expect(list[2]).toMatchObject({ rank: 2 });
  });

  it("keeps the lists short", () => {
    const recents = Array.from({ length: 12 }, (_, index) => recent(`r${index}`, `q${index}`));
    const trends = Array.from({ length: 12 }, (_, index) => trend(`t${index}`, index + 1));
    const list = idleSuggestions(recents, trends, { recent: 3, trending: 2 });
    expect(list.filter((item) => item.group === "recent")).toHaveLength(3);
    expect(list.filter((item) => item.group === "trending")).toHaveLength(2);
  });
});
