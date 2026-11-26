import { defaultParseSearch } from "@tanstack/react-router";
import { search } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { hitHref } from "./hit-href";
import { parseSearchParams } from "./params";

const K = search.SearchKind;

const hit = (kind: search.SearchKind, path: string, title: string, extra: object = {}) => ({
  kind,
  id: "01a200b9-ba8c-7882-b900-6820e398de5d",
  path,
  title,
  extraJson: JSON.stringify(extra),
  owner: undefined,
});

describe("where a result leads", () => {
  it("follows the hit's path", () => {
    expect(hitHref(hit(K.USER, "/u/old", "さくら", { handle: "sakura" }))).toBe("/u/sakura");
    expect(hitHref(hit(K.VIDEO, "/watch/abc123", "ねこ動画"))).toBe("/watch/abc123");
  });

  it("searches for a hit whose path leaves this site, in its own tab", () => {
    for (const path of ["https://evil.example/x", "//evil.example/x", "watch/abc", ""]) {
      const href = hitHref(hit(K.VIDEO, path, "ねこ動画"));
      expect(href.startsWith("/search?"), path).toBe(true);
      expect(parseSearchParams(defaultParseSearch(href.slice(href.indexOf("?"))))).toMatchObject({
        q: "ねこ動画",
        kind: "videos",
      });
    }
    expect(
      hitHref(hit(K.THREAD, "//evil.example", "t", { community_handle: "x" })).startsWith(
        "/search?",
      ),
    ).toBe(true);
  });
});
