import { afterEach, describe, expect, it, vi } from "vitest";
import { forgetSignedOutAccount, KEPT_STORAGE, type SignOutCleanup } from "./sign-out";

/** The storage keys (or how they start) named by the app's code that touches browser storage. */
function storageKeysInCode(): Map<string, string> {
  const sources = import.meta.glob<string>(["../**/*.ts", "../**/*.tsx", "!../**/*.test.*"], {
    query: "?raw",
    import: "default",
    eager: true,
  });
  const keys = new Map<string, string>();
  for (const [path, source] of Object.entries(sources)) {
    if (!/localStorage|sessionStorage|DeviceMemory|GiftStore/.test(source)) continue;
    for (const [, key] of source.matchAll(/["'`](tokuly[._][A-Za-z0-9._:-]*)/g)) {
      // Protobuf names, file names and the server's modules are no storage keys.
      if (!key || /\.v\d|\.toml$|::/.test(key)) continue;
      keys.set(key, path);
    }
  }
  return keys;
}

/** A browser's storage. */
function storage(): Storage {
  const values = new Map<string, string>();
  return {
    get length() {
      return values.size;
    },
    key: (index: number) => [...values.keys()][index] ?? null,
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => void values.set(key, value),
    removeItem: (key: string) => void values.delete(key),
    clear: () => values.clear(),
  };
}

/** A storage whose own keys are its entries (`Object.keys(localStorage)` in a browser). */
function browserStorage(): Storage {
  const backing = storage();
  return new Proxy(backing, {
    ownKeys: () => Array.from({ length: backing.length }, (_, i) => backing.key(i) ?? ""),
    getOwnPropertyDescriptor: (target, key) =>
      typeof key === "string" && backing.getItem(key) !== null
        ? { value: backing.getItem(key), enumerable: true, configurable: true }
        : Reflect.getOwnPropertyDescriptor(target, key),
  });
}

const cleanup = (forget: () => void) => async () => ({
  default: { forget } satisfies SignOutCleanup,
});

describe("signing out", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  it("keeps only entries the app's code still names", () => {
    const named = [...storageKeysInCode().keys()];
    const stale = Object.keys(KEPT_STORAGE).filter((entry) => !named.includes(entry));
    expect(stale).toEqual([]);
  });

  it("leaves nothing of the previous account in this browser's storage", async () => {
    const local = browserStorage();
    const session = browserStorage();
    vi.stubGlobal("window", { localStorage: local, sessionStorage: session });
    // Everything the code may write, as an account would: per-account keys end in its ID.
    for (const key of storageKeysInCode().keys()) {
      const name = /[.:]$/.test(key) ? `${key}user-1` : key;
      local.setItem(name, "1");
      session.setItem(name, "1");
    }
    await forgetSignedOutAccount([]);

    for (const left of [local, session]) {
      const kept = Object.keys(left).filter(
        (key) =>
          !(key in KEPT_STORAGE) &&
          !Object.keys(KEPT_STORAGE).some((start) => start.endsWith(".") && key.startsWith(start)),
      );
      expect(kept).toEqual([]);
    }
    expect(local.getItem("tokuly.theme")).toBe("1");
    expect(local.getItem("tokuly.map.stopOwed.user-1")).toBe("1");
    expect(local.getItem("tokuly.emoji.showSensitive")).toBeNull();
  });

  it("goes on when one cleanup fails", async () => {
    vi.stubGlobal("window", { localStorage: browserStorage(), sessionStorage: browserStorage() });
    vi.spyOn(console, "error").mockImplementation(() => {});
    const after = vi.fn();
    await forgetSignedOutAccount([
      cleanup(() => {
        throw new Error("a broken feature");
      }),
      cleanup(after),
    ]);
    expect(after).toHaveBeenCalledOnce();
  });

  it("tries a cleanup that did not arrive again", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("window", { localStorage: browserStorage(), sessionStorage: browserStorage() });
    const forget = vi.fn();
    const load = vi
      .fn<() => Promise<{ default: SignOutCleanup }>>()
      .mockRejectedValueOnce(new TypeError("Failed to fetch dynamically imported module"))
      .mockImplementation(cleanup(forget));
    const done = forgetSignedOutAccount([load]);
    await vi.runAllTimersAsync();
    await done;
    expect(forget).toHaveBeenCalledOnce();
  });

  it("starts the page anew when a cleanup never arrives, after removing the account's storage", async () => {
    vi.useFakeTimers();
    vi.spyOn(console, "error").mockImplementation(() => {});
    const local = browserStorage();
    const replace = vi.fn();
    vi.stubGlobal("window", {
      localStorage: local,
      sessionStorage: browserStorage(),
      location: { replace },
    });
    local.setItem("tokuly.chat.draft.c1", "ひみつ");
    const loaded = vi.fn();
    let returned = false;
    void forgetSignedOutAccount([
      () => Promise.reject(new TypeError("Failed to fetch dynamically imported module")),
      cleanup(loaded),
    ]).then(() => (returned = true));
    expect(local.getItem("tokuly.chat.draft.c1")).toBeNull();
    await vi.runAllTimersAsync();
    expect(loaded).toHaveBeenCalledOnce();
    expect(replace).toHaveBeenCalledWith("/");
    expect(returned).toBe(false);
  });
});
