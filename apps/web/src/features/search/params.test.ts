import { defaultParseSearch } from "@tanstack/react-router";
import { search } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { fromTagQuery, isTagQuery, MAX_QUERY_LENGTH, parseSearchParams, searchHref, sortValue, toTagQuery } from "./params";

describe("the address of a result page", () => {
  it("reads query, tab, order and owner", () => {
    expect(parseSearchParams({ q: " さくら ", kind: "notes", sort: "newest", owner: "u1" })).toEqual({ q: "さくら", kind: "notes", sort: "newest", owner: "u1" });
  });

  it("is the explore page without a query", () => {
    expect(parseSearchParams({})).toEqual({ q: undefined, kind: undefined, sort: undefined, owner: undefined });
    expect(parseSearchParams({ q: "   " }).q).toBeUndefined();
  });

  it("keeps queries the router parsed into something other than text", () => {
    expect(parseSearchParams({ q: 2026 }).q).toBe("2026");
    expect(parseSearchParams({ q: true }).q).toBe("true");
    expect(parseSearchParams({ q: null }).q).toBe("null");
  });

  it("drops tabs and orders it does not know, and cuts overlong queries", () => {
    expect(parseSearchParams({ q: "a", kind: "everything", sort: "oldest" })).toEqual({ q: "a", kind: undefined, sort: undefined, owner: undefined });
    expect(parseSearchParams({ q: "あ".repeat(MAX_QUERY_LENGTH + 20) }).q).toHaveLength(MAX_QUERY_LENGTH);
  });

  /** What the router hands the route for an address. */
  const read = (href: string) => parseSearchParams(defaultParseSearch(href.slice(href.indexOf("?"))));

  it("round-trips through searchHref", () => {
    const href = searchHref("カフェ 渋谷", { kind: search.SearchKind.NOTE, sort: "popular", owner: "u1" });
    expect(href.startsWith("/search?")).toBe(true);
    expect(read(href)).toEqual({ q: "カフェ 渋谷", kind: "notes", sort: "popular", owner: "u1" });
    expect(read(searchHref("#散歩")).q).toBe("#散歩");
  });

  it("keeps queries that look like JSON exactly as typed", () => {
    for (const query of ['"foo bar"', '"さくら ラテ" 渋谷', "2026", "true", "null", '{"a":1}', "[1,2]", "a&b=c"]) expect(read(searchHref(query)).q, query).toBe(query);
  });

  it("maps the order to the API's", () => {
    expect(sortValue(undefined)).toBe(search.SearchSort.RELEVANCE);
    expect(sortValue("newest")).toBe(search.SearchSort.NEWEST);
    expect(sortValue("popular")).toBe(search.SearchSort.POPULAR);
  });
});

describe("tag-only queries", () => {
  it("are recognized by a leading '#', half or full width", () => {
    expect(isTagQuery("#さんぽ")).toBe(true);
    expect(isTagQuery(" ＃さんぽ")).toBe(true);
    expect(isTagQuery("さんぽ #犬")).toBe(false);
  });

  it("switch on and off without piling up '#'", () => {
    expect(toTagQuery("さんぽ")).toBe("#さんぽ");
    expect(toTagQuery("##さんぽ")).toBe("#さんぽ");
    expect(fromTagQuery("＃さんぽ")).toBe("さんぽ");
    expect(fromTagQuery(toTagQuery("さんぽ"))).toBe("さんぽ");
  });
});
