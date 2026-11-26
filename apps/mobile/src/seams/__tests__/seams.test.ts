import { targetOf } from "#/features/notifications/target";
import { SERVER_URL } from "#/lib/config";
import { HUB_ENTRIES } from "../hub";
import { screenFor } from "../links";
import { SCREENS } from "../screens";
import { SETTINGS_CARDS, SETTINGS_ROWS } from "../settings";
import { TABS } from "../tabs";

// The rows only name the router; no navigation is mounted here.
jest.mock("expo-router", () => ({ router: { push: jest.fn() } }));

/** What the features plug in today: moving them onto the seams changed nothing a person sees. */
describe("the features on the seams", () => {
  it("the tabs, in the bar's order, with their counters", () => {
    expect(TABS.map((tab) => [tab.name, tab.badge, tab.noHeader ?? false])).toEqual([
      ["map", undefined, true],
      ["chats", "chat", false],
      ["notes", undefined, false],
      ["shorts", undefined, true],
      ["friends", "friends", false],
      ["inbox", "notifications", false],
      ["me", undefined, false],
    ]);
    expect(TABS.map((tab) => tab.title())).toEqual([
      "マップ",
      "チャット",
      "ノート",
      "ショート",
      "ともだち",
      "お知らせ",
      "じぶん",
    ]);
  });

  it("the hub and the settings", () => {
    expect(HUB_ENTRIES.map((entry) => [entry.id, entry.href])).toEqual([
      ["video", "/watch"],
      ["more", "/more"],
      ["settings", "/settings"],
    ]);
    expect(SETTINGS_ROWS.map((row) => row.id)).toEqual(["accounts", "web", "licenses"]);
    expect(SETTINGS_CARDS.map((card) => card.id)).toEqual(["push"]);
  });

  it("the screens pushed over the tabs, each named once", () => {
    const names = SCREENS.map((screen) => screen.name).sort();
    expect(names).toEqual([
      "accounts",
      "add-account",
      "add-friend",
      "c/[handle]",
      "chat/[id]",
      "create-account",
      "invite/[code]",
      "licenses",
      "more",
      "new-chat",
      "notes/[id]",
      "notes/antenna/[id]",
      "notes/clips/[id]",
      "notes/clips/index",
      "notes/compose",
      "notes/drafts",
      "notes/list/[id]",
      "notes/tag/[tag]",
      "notes/user/[id]",
      "playlist/[id]",
      "settings",
      "shorts/mine",
      "shorts/new",
      "shorts/sound/[id]",
      "shorts/tag/[tag]",
      "shorts/watch",
      "stories/[userId]",
      "stories/new",
      "u/[handle]",
      "watch/[videoId]",
      "watch/channels",
      "watch/history",
      "watch/index",
      "watch/library",
      "web",
    ]);
    expect(SCREENS.find((screen) => screen.name === "new-chat")?.options()).toEqual({
      title: "新しいチャット",
      presentation: "modal",
    });
  });

  it("the server's paths that have a screen here", () => {
    expect(screenFor("/chat/c1")).toBe("/chat/c1");
    expect(screenFor("/u/alice")).toBe("/u/alice");
    expect(screenFor("/invite/abc")).toBe("/invite/abc");
    expect(screenFor("/friends/requests")).toBe("/friends");
    expect(screenFor("/map")).toBe("/map");
    expect(screenFor("/notifications")).toBe("/inbox");
    expect(screenFor("/notes/n1")).toBe("/notes/n1");
    expect(screenFor("/notes/tags/ramen")).toBe("/notes/tag/ramen");
    // What only the web has opens in the app's web view, signed in.
    expect(screenFor("/stamps")).toBe(`/web?path=${encodeURIComponent("/stamps")}`);
    expect(screenFor("/notes/channels")).toBe(`/web?path=${encodeURIComponent("/notes/channels")}`);
    // What no feature claims opens on the web.
    expect(screenFor("/login")).toBeNull();
    expect(targetOf("/chat/c1?m=5")).toEqual({ type: "screen", href: "/chat/c1" });
    expect(targetOf("/login")).toEqual({ type: "web", url: `${SERVER_URL}/login` });
  });
});
