import { targetOf } from "#/features/notifications/target";
import { SERVER_URL } from "#/lib/config";
import { HUB_ENTRIES, waitingInHub } from "../hub";
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
      ["inbox", "notifications", false],
      ["me", "hub", false],
    ]);
    // A seventh area goes to the hub, or takes the place of one that moves there.
    expect(TABS.length).toBeLessThanOrEqual(6);
    expect(TABS.map((tab) => tab.title())).toEqual([
      "マップ",
      "チャット",
      "ノート",
      "ショート",
      "お知らせ",
      "じぶん",
    ]);
  });

  it("the hub and the settings", () => {
    expect(HUB_ENTRIES.map((entry) => [entry.id, entry.href])).toEqual([
      ["friends", "/friends"],
      ["search", "/search"],
      ["video", "/watch"],
      ["live", "/live"],
      ["more", "/more"],
      ["settings", "/settings"],
    ]);
    // What waits behind an entry (a friend request) still shows on the bar, on the hub's tab.
    expect(waitingInHub({ friends: 2, chat: 5 })).toBe(2);
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
      "friends",
      "invite/[code]",
      "licenses",
      "live/[id]",
      "live/index",
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
      "search",
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
    expect(screenFor("/live")).toBe("/live");
    expect(screenFor("/live/abc123")).toBe("/live/abc123");
    expect(screenFor("/live/gifts")).toBe(`/web?path=${encodeURIComponent("/live/gifts")}`);
    expect(screenFor("/live/clips/new")).toBe(`/web?path=${encodeURIComponent("/live/clips/new")}`);
    // What only the web has opens in the app's web view, signed in.
    expect(screenFor("/stamps")).toBe(`/web?path=${encodeURIComponent("/stamps")}`);
    expect(screenFor("/notes/channels")).toBe(`/web?path=${encodeURIComponent("/notes/channels")}`);
    // What no feature claims opens on the web.
    expect(screenFor("/login")).toBeNull();
    expect(targetOf("/chat/c1?m=5")).toEqual({ type: "screen", href: "/chat/c1?m=5" });
    expect(targetOf("/login")).toEqual({ type: "web", url: `${SERVER_URL}/login` });
  });

  it("keep the link's query and fragment", () => {
    expect(screenFor("/watch/abc123?t=90")).toBe("/watch/abc123?t=90");
    expect(screenFor("/notes/n1?comment=c9#c9")).toBe("/notes/n1?comment=c9#c9");
    expect(screenFor("/notifications?tab=mentions")).toBe("/inbox?tab=mentions");
    // The web view opens the whole link, its query and fragment included.
    expect(screenFor("/stamps?tab=new#top")).toBe(
      `/web?path=${encodeURIComponent("/stamps?tab=new#top")}`,
    );
    // A page a feature leaves to the web keeps them inside the page's own path, like any web-only one.
    expect(screenFor("/notes/channels?tab=mine#top")).toBe(
      `/web?path=${encodeURIComponent("/notes/channels?tab=mine#top")}`,
    );
    expect(screenFor("/live/gifts?month=2026-11")).toBe(
      `/web?path=${encodeURIComponent("/live/gifts?month=2026-11")}`,
    );
    // The page the web view opens is the link's, never one its query names.
    expect(screenFor("/live/clips/new?path=/settings")).toBe(
      `/web?path=${encodeURIComponent("/live/clips/new?path=/settings")}`,
    );
    expect(screenFor("/live/abc123?t=90")).toBe("/live/abc123?t=90");
    expect(screenFor("/login?next=/map")).toBeNull();
    expect(targetOf("/watch/abc123?t=90")).toEqual({ type: "screen", href: "/watch/abc123?t=90" });
  });
});
