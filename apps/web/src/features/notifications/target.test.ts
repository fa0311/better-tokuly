import { describe, expect, it } from "vitest";
import { resolveTargetPath } from "./target";

const ORIGIN = "https://tokuly.example";
const resolve = (path: string) => resolveTargetPath(path, ORIGIN);

describe("notification targets", () => {
  it("keeps a path inside the app, with its search and hash", () => {
    expect(resolve("/chat/42")).toEqual({ type: "internal", href: "/chat/42" });
    expect(resolve("/u/alice?tab=notes#top")).toEqual({
      type: "internal",
      href: "/u/alice?tab=notes#top",
    });
    expect(resolve("  /map/ ")).toEqual({ type: "internal", href: "/map" });
    expect(resolve("/")).toEqual({ type: "internal", href: "/" });
  });

  it("treats a full URL of this site as a path", () => {
    expect(resolve("https://tokuly.example/watch/7?t=30")).toEqual({
      type: "internal",
      href: "/watch/7?t=30",
    });
  });

  it("sends the test push's path to the notification settings", () => {
    expect(resolve("/settings/notifications")).toEqual({
      type: "internal",
      href: "/notifications/settings",
    });
    expect(resolve("https://tokuly.example/settings/notifications/")).toEqual({
      type: "internal",
      href: "/notifications/settings",
    });
  });

  it("recognizes links that leave the site", () => {
    expect(resolve("https://example.com/news")).toEqual({
      type: "external",
      url: "https://example.com/news",
    });
    expect(resolve("//example.com/news")).toEqual({
      type: "external",
      url: "https://example.com/news",
    });
  });

  it("goes nowhere for an empty path or a script URL", () => {
    expect(resolve("")).toBeNull();
    expect(resolve("   ")).toBeNull();
    expect(resolve("javascript:alert(1)")).toBeNull();
    expect(resolve("data:text/html,hi")).toBeNull();
  });
});
