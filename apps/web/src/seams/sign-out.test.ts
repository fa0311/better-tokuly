import { afterEach, describe, expect, it, vi } from "vitest";
import { forgetSignedOutAccount, type SignOutCleanup, signOutCleanups } from "./sign-out";

/**
 * What this browser keeps on purpose after a sign-out, and why. Every other
 * storage key the app's code names must be some feature's `sign-out.ts`.
 */
const KEPT: Record<string, string> = {
  "tokuly.deviceId": "this browser, not an account",
  tokuly_lang: "the browser's language",
  "tokuly.theme": "the look of this browser",
  "tokuly.dopa": "the look of this browser",
  "tokuly.colors": "the look of this browser (the default again on sign-out)",
  "tokuly.signedIn": "the session's own hint, removed by stores/session.ts",
  "tokuly.accountChanged": "tabs telling each other of a switch",
  "tokuly.account": "the channel tabs tell each other on (no storage)",
  tokuly_session_hint: "the app's hand-off cookie, taken at once",
  "tokuly.push.registration": "the push subscription, taken back before the sign-out",
  "tokuly.push.promptDismissedAt": "this browser's answer to the prompt",
  "tokuly.pwa.installDismissedAt": "this browser's answer to the prompt",
  "tokuly.map.sessionEnding": "owed: a stop of sharing not yet sent",
  "tokuly.map.sharing.": "owed: a stop of sharing not yet sent, by account",
  "tokuly.map.stopOwed.": "owed: a stop of sharing not yet sent, by account",
  "tokuly.map.sequence.": "owed: the order of the account's reports",
  "tokuly.live.gift.": "owed: an unanswered gift, by account",
  "tokuly.party.guestName": "a guest's seat at a watch party, without an account",
  "tokuly.party.guest.": "a guest's seat at a watch party, without an account",
  "tokuly.spaces.microphone": "this device's microphone",
  "tokuly.spaces.speaker": "this device's speaker",
  "tokuly.shorts.muted": "this device's sound",
  "tokuly.player.theatre": "this device's player",
  "tokuly.player.volume": "this device's player",
  "tokuly.player.muted": "this device's player",
  "tokuly.player.captions": "this device's player",
  "tokuly.emoji": "the emoji catalogue, the same for everyone",
};

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

const covered = (key: string, starts: readonly string[]) =>
  starts.some((start) => key.startsWith(start) || start.startsWith(key));

describe("signing out", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("knows every storage key of the app: kept on purpose, or removed with the account", async () => {
    const starts = (await signOutCleanups()).flatMap(({ storage }) => storage ?? []);
    const unaccounted = [...storageKeysInCode()]
      .filter(([key]) => !(key in KEPT) && !covered(key, starts))
      .map(([key, path]) => `${key} (${path})`);
    expect(unaccounted).toEqual([]);
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
    const storageOnly = (await signOutCleanups()).map(({ storage }) => ({ storage }));
    await forgetSignedOutAccount(storageOnly);

    for (const left of [local, session]) {
      const kept = Object.keys(left).filter(
        (key) => !Object.keys(KEPT).some((start) => key === start || key.startsWith(start)),
      );
      expect(kept).toEqual([]);
    }
    expect(local.getItem("tokuly.theme")).toBe("1");
  });

  it("goes on when one cleanup fails", async () => {
    vi.stubGlobal("window", { localStorage: browserStorage(), sessionStorage: browserStorage() });
    vi.spyOn(console, "error").mockImplementation(() => {});
    const after = vi.fn();
    const cleanups: SignOutCleanup[] = [
      {
        forget: () => {
          throw new Error("a broken feature");
        },
      },
      { forget: after },
    ];
    await forgetSignedOutAccount(cleanups);
    expect(after).toHaveBeenCalledOnce();
  });
});
