import type { SessionState } from "@tokuly/api";
import { afterEach, describe, expect, it, vi } from "vitest";

const listeners = vi.hoisted(() => new Set<(state: SessionState) => void>());
vi.mock("#/lib/api", () => ({
  api: () => ({
    session: {
      subscribe(listener: (state: SessionState) => void) {
        listeners.add(listener);
        return () => listeners.delete(listener);
      },
    },
  }),
}));
vi.mock("react", () => ({
  // The effect runs at once and its clean-up is kept for the test.
  useEffect(effect: () => () => void) {
    cleanups.push(effect());
  },
}));
const cleanups: (() => void)[] = [];
const stored = new Map<string, string>();
vi.stubGlobal("window", {
  localStorage: {
    getItem: (key: string) => stored.get(key) ?? null,
    setItem: (key: string, value: string) => void stored.set(key, value),
    removeItem: (key: string) => void stored.delete(key),
  },
});

const { LAST_VIEW_KEY, useForgetLastViewOnSignOut } = await import("./last-view");
const becomes = (state: SessionState) => {
  for (const listener of listeners) listener(state);
};

describe("where the map was left", () => {
  afterEach(() => {
    stored.clear();
    listeners.clear();
  });

  it("is forgotten when the session ends, and nothing else is", () => {
    stored.set(LAST_VIEW_KEY, JSON.stringify({ center: [139.7, 35.66], zoom: 15 }));
    stored.set("tokuly.deviceId", "device");
    useForgetLastViewOnSignOut();
    // A refreshed token or a changed profile is the same session.
    becomes({ status: "signedIn", sessionId: "s1", me: {} } as SessionState);
    becomes({ status: "unknown" });
    expect(stored.has(LAST_VIEW_KEY)).toBe(true);

    // Signed out here, from another device, or by the server: all the same.
    becomes({ status: "signedOut" });
    expect([...stored.keys()]).toEqual(["tokuly.deviceId"]);
  });

  it("stops listening when the background is gone", () => {
    useForgetLastViewOnSignOut();
    for (const cleanup of cleanups.splice(0)) cleanup();
    expect(listeners.size).toBe(0);
  });
});
