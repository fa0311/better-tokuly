import { describe, expect, it } from "vitest";
import { notificationPath, resolveTarget } from "./target";

const ORIGIN = "https://tokuly.example";
const resolve = (path: string) => resolveTarget(path, ORIGIN);

describe("notification targets", () => {
  it("keeps a path inside the app, with its search and hash", () => {
    expect(resolve("/chat/42")).toEqual({ type: "internal", href: "/chat/42" });
    expect(resolve("/u/alice?tab=notes#top")).toEqual({
      type: "internal",
      href: "/u/alice?tab=notes#top",
    });
    expect(resolve("  /map/ ")).toEqual({ type: "internal", href: "/map" });
  });

  it("treats a full URL of the app's server as a path", () => {
    expect(resolve("https://tokuly.example/watch/7?t=30")).toEqual({
      type: "internal",
      href: "/watch/7?t=30",
    });
    expect(resolveTarget("http://10.0.2.2/chat/1", "http://10.0.2.2/")).toEqual({
      type: "internal",
      href: "/chat/1",
    });
  });

  it("recognizes links that leave the server, and refuses other schemes", () => {
    expect(resolve("https://example.com/news")).toEqual({
      type: "external",
      url: "https://example.com/news",
    });
    expect(resolve("javascript:alert(1)")).toBeNull();
    expect(resolve("   ")).toBeNull();
  });
});

describe("notification paths", () => {
  it("sends community notifications to the community pages, not to a video channel", () => {
    expect(notificationPath("community.reply", "/c/neko/t/bXou6FpLkfK")).toBe(
      "/g/neko/t/bXou6FpLkfK",
    );
    expect(notificationPath("community.join_approved", "/c/neko")).toBe("/g/neko");
    expect(notificationPath("community.review", "/c/neko/review")).toBe("/g/neko/manage/queue");
    expect(notificationPath("community.event_reminder", "/c/neko/events/0199-ab")).toBe(
      "/g/neko/events/0199-ab",
    );
  });

  it("leaves every other path alone, and a community path it cannot read", () => {
    expect(notificationPath("video.comment", "/c/someone")).toBe("/c/someone");
    expect(notificationPath(undefined, "/c/someone")).toBe("/c/someone");
    expect(notificationPath("community.review", "/c/neko/videos")).toBe("/c/neko/videos");
  });
});
