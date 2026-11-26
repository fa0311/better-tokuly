import { afterEach, describe, expect, it, vi } from "vitest";

const hooks = vi.hoisted(() => new Set<() => void>());
vi.mock("#/stores/session", () => ({
  onBeforeSignOut(hook: () => void) {
    hooks.add(hook);
    return () => hooks.delete(hook);
  },
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

describe("where the map was left", () => {
  afterEach(() => {
    stored.clear();
    hooks.clear();
  });

  it("is forgotten when the account signs out, and nothing else is", () => {
    stored.set(LAST_VIEW_KEY, JSON.stringify({ center: [139.7, 35.66], zoom: 15 }));
    stored.set("tokuly.deviceId", "device");
    useForgetLastViewOnSignOut();
    expect(stored.has(LAST_VIEW_KEY)).toBe(true);

    for (const hook of hooks) hook();
    expect([...stored.keys()]).toEqual(["tokuly.deviceId"]);
  });

  it("stops listening when the background is gone", () => {
    useForgetLastViewOnSignOut();
    for (const cleanup of cleanups.splice(0)) cleanup();
    expect(hooks.size).toBe(0);
  });
});
