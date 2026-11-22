import { search } from "@tokuly/api";
import { defaultParseSearch } from "@tanstack/react-router";
import { describe, expect, it } from "vitest";
import { hitHref, isAppPath } from "./hit-href";
import { hitExtra, isKind, KINDS, kindFromSlug, kindSlug } from "./kinds";
import { parseSearchParams } from "./params";

const K = search.SearchKind;

/** A hit with only what `hitHref` looks at. */
function hit(kind: search.SearchKind, fields: { id?: string; path?: string; title?: string; extra?: object | string; ownerHandle?: string } = {}) {
  return {
    kind,
    id: fields.id ?? "01a200b9-ba8c-7882-b900-6820e398de5d",
    path: fields.path ?? "",
    title: fields.title ?? "",
    extraJson: typeof fields.extra === "string" ? fields.extra : JSON.stringify(fields.extra ?? {}),
    owner: fields.ownerHandle ? ({ handle: fields.ownerHandle } as search.SearchHit["owner"]) : undefined,
  };
}

describe("search kinds", () => {
  it("has a tab for every kind the API defines", () => {
    const defined = Object.values(K).filter((value): value is search.SearchKind => typeof value === "number" && value !== K.UNSPECIFIED);
    expect([...KINDS].sort((a, b) => a - b)).toEqual(defined.sort((a, b) => a - b));
  });

  it("names every kind in the address, and reads the name back", () => {
    for (const kind of KINDS) expect(kindFromSlug(kindSlug(kind))).toBe(kind);
    expect(new Set(KINDS.map(kindSlug)).size).toBe(KINDS.length);
    expect(kindFromSlug("people")).toBe(K.USER);
    expect(kindFromSlug("everything")).toBeUndefined();
    expect(kindFromSlug(3)).toBeUndefined();
    expect(isKind(K.UNSPECIFIED)).toBe(false);
  });
});

describe("where a result leads", () => {
  it("sends people to their profile, by the handle the search domain published", () => {
    expect(hitHref(hit(K.USER, { path: "/u/old_handle", extra: { handle: "sakura" } }))).toBe("/u/sakura");
    expect(hitHref(hit(K.USER, { path: "/u/sakura", ownerHandle: "sakura" }))).toBe("/u/sakura");
    expect(hitHref(hit(K.USER, { path: "/u/sakura" }))).toBe("/u/sakura");
  });

  it("uses this app's addresses for channels, notes, hashtags and news", () => {
    expect(hitHref(hit(K.CHANNEL, { path: "/channel/tokuly_tv", extra: { handle: "tokuly_tv" } }))).toBe("/c/tokuly_tv");
    expect(hitHref(hit(K.NOTE, { id: "n1", path: "/notes/n1" }))).toBe("/notes/n1");
    expect(hitHref(hit(K.HASHTAG, { path: "/notes/tags/さくら", title: "#さくら", extra: { tag: "さくら" } }))).toBe(`/tag/${encodeURIComponent("さくら")}`);
    expect(hitHref(hit(K.HASHTAG, { path: "/notes/tags/cafe", title: "＃cafe" }))).toBe("/tag/cafe");
    expect(hitHref(hit(K.NEWS, { id: "a1", path: "/news/articles/a1" }))).toBe("/news/a1");
  });

  it("uses this app's addresses for emoji, communities and their threads", () => {
    expect(hitHref(hit(K.EMOJI, { path: "/emoji/party_parrot", extra: { name: "party_parrot" } }))).toBe("/emoji/party_parrot");
    expect(hitHref(hit(K.EMOJI, { path: "/emoji/old_name", title: "ねこ", extra: { name: "ねこ" } }))).toBe(`/emoji/${encodeURIComponent("ねこ")}`);
    // `/c/<handle>` is a video channel here: communities live under `/g`.
    expect(hitHref(hit(K.COMMUNITY, { path: "/c/photo_club", extra: { handle: "photo_club" } }))).toBe("/g/photo_club");
    expect(hitHref(hit(K.THREAD, { path: "/c/photo_club/t/junEDItkYwO", extra: { community_handle: "photo_club" } }))).toBe("/g/photo_club/t/junEDItkYwO");
    // Without what the address is built from, the published path is the best there is.
    expect(hitHref(hit(K.COMMUNITY, { path: "/c/photo_club" }))).toBe("/c/photo_club");
    expect(hitHref(hit(K.THREAD, { path: "/somewhere/else", extra: { community_handle: "photo_club" } }))).toBe("/somewhere/else");
  });

  it("opens answers and audio rooms where their domains published them", () => {
    expect(hitHref(hit(K.ANSWER, { id: "q1", path: "/ask/q/q1" }))).toBe("/ask/q/q1");
    expect(hitHref(hit(K.SPACE, { id: "s1", path: "/spaces/s1" }))).toBe("/spaces/s1");
    expect(hitHref(hit(K.THREAD, { path: "//evil.example", title: "t", extra: { community_handle: "x" } })).startsWith("/search?")).toBe(true);
  });

  it("opens everything else at the path its domain published", () => {
    expect(hitHref(hit(K.VIDEO, { path: "/watch/abc123" }))).toBe("/watch/abc123");
    expect(hitHref(hit(K.STAMP_PACK, { path: "/stamps/pk9" }))).toBe("/stamps/pk9");
    expect(hitHref(hit(K.PLAYLIST, { path: "/playlist/pl1" }))).toBe("/playlist/pl1");
    expect(hitHref(hit(K.PLACE, { path: "/places/p1" }))).toBe("/places/p1");
    // A channel document without a handle still leads somewhere.
    expect(hitHref(hit(K.CHANNEL, { path: "/channel/x" }))).toBe("/channel/x");
  });

  it("survives extra fields that are not an object", () => {
    expect(hitExtra({ extraJson: "" })).toEqual({});
    expect(hitExtra({ extraJson: "not json" })).toEqual({});
    expect(hitExtra({ extraJson: "[1,2]" })).toEqual({});
    expect(hitExtra({ extraJson: "null" })).toEqual({});
    expect(hitHref(hit(K.USER, { path: "/u/a", extra: "{broken" }))).toBe("/u/a");
  });

  it("never follows a published path that leaves this site", () => {
    for (const path of ["https://evil.example/x", "//evil.example/x", "/\\evil.example", "javascript:alert(1)", "watch/abc", ""]) {
      expect(isAppPath(path), path).toBe(false);
      const href = hitHref(hit(K.VIDEO, { path, title: "ねこ動画" }));
      expect(href.startsWith("/search?"), path).toBe(true);
      expect(parseSearchParams(defaultParseSearch(href.slice(href.indexOf("?"))))).toMatchObject({ q: "ねこ動画", kind: "videos" });
    }
    expect(isAppPath("/watch/abc?t=3")).toBe(true);
    // The same goes for kinds whose own address cannot be built.
    expect(hitHref(hit(K.CHANNEL, { path: "//evil.example", title: "ch" })).startsWith("/search?")).toBe(true);
  });
});
