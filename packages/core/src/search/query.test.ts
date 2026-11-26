import { search } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { fromTagQuery, isTagQuery, sortValue, toTagQuery } from "./query";

describe("result order", () => {
  it("maps to the API's", () => {
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
