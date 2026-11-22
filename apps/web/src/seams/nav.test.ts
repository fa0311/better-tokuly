import { describe, expect, it } from "vitest";
import { type PrimaryNav, primaryNavOf } from "./nav";

const claims: { path: string; primary: PrimaryNav }[] = [
  { path: "/watch/history", primary: "me" },
  { path: "/notes", primary: "notes" },
  { path: "/watch", primary: "watch" },
  { path: "/tag", primary: "notes" },
  { path: "/c", primary: "watch" },
];

describe("primaryNavOf", () => {
  it("gives a claimed path and everything below it to its primary item", () => {
    expect(primaryNavOf("/notes", claims)).toBe("notes");
    expect(primaryNavOf("/tag/散歩", claims)).toBe("notes");
    expect(primaryNavOf("/c/tokuly/videos", claims)).toBe("watch");
  });

  it("does not match a path that merely starts with the same letters", () => {
    expect(primaryNavOf("/chat", claims)).toBe("me");
    expect(primaryNavOf("/tags", claims)).toBe("me");
  });

  it("prefers the longest claim", () => {
    expect(primaryNavOf("/watch/history", claims)).toBe("me");
    expect(primaryNavOf("/watch/abc", claims)).toBe("watch");
  });

  it("sends unclaimed pages to the hub", () => {
    expect(primaryNavOf("/wallet/buy", claims)).toBe("me");
  });

  it("knows the claims of the features", () => {
    expect(primaryNavOf("/stories/archive")).toBe("notes");
    expect(primaryNavOf("/bookmarks")).toBe("notes");
    expect(primaryNavOf("/party/abc")).toBe("watch");
    expect(primaryNavOf("/places/new")).toBe("map");
    expect(primaryNavOf("/chat/123")).toBe("chat");
  });
});
