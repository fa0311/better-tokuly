import { type CatalogEmoji, searchEmoji } from "@tokuly/core/emoji";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createCatalogStore, STORE_TIMEOUT_MS } from "./catalog-store";
import { UNICODE_EMOJI, UNICODE_GROUPS } from "./unicode-data";

function custom(name: string, fields: Partial<CatalogEmoji> = {}): CatalogEmoji {
  return {
    id: `id-${name}`,
    name,
    url: `/${name}.webp`,
    stills: [],
    animated: false,
    category: "",
    aliases: [],
    sensitive: false,
    localOnly: false,
    addedAt: 0,
    ...fields,
  };
}

describe("searchEmoji with the Unicode table", () => {
  const customs = [
    custom("neko", { aliases: ["ねこ", "cat"] }),
    custom("neko_love", { aliases: ["猫"] }),
    custom("inu", { aliases: ["いぬ", "catdog"] }),
  ];

  it("ranks exact matches before prefixes before substrings, custom before Unicode", () => {
    const found = searchEmoji("cat", customs, UNICODE_EMOJI).map((choice) => choice.emoji);
    expect(found.slice(0, 2)).toEqual([":neko:", "🐱"]);
    expect(found).toContain(":inu:");
    expect(found.indexOf(":inu:")).toBeGreaterThan(found.indexOf("🐱"));
  });

  it("finds Unicode emoji in Japanese and English", () => {
    expect(searchEmoji("ねこ", [], UNICODE_EMOJI)[0]?.emoji).toBe("🐱");
    expect(searchEmoji("すし", [], UNICODE_EMOJI)[0]?.emoji).toBe("🍣");
    expect(searchEmoji("thumbsup", [], UNICODE_EMOJI)[0]?.emoji).toBe("👍");
  });
});

describe("the Unicode table", () => {
  it("lists every emoji once, each with words in both languages", () => {
    const seen = new Set<string>();
    for (const entry of UNICODE_EMOJI) {
      expect(seen.has(entry.emoji), entry.emoji).toBe(false);
      seen.add(entry.emoji);
      expect(entry.en.length, entry.emoji).toBeGreaterThan(0);
      expect(entry.ja.length, entry.emoji).toBeGreaterThan(0);
    }
    expect(UNICODE_GROUPS.every((group) => group.emoji.length > 0)).toBe(true);
  });
});

describe("the stored catalogue copy", () => {
  afterEach(() => vi.useRealTimers());

  /** An IndexedDB whose `open` hands out requests that the test settles by hand (or never). */
  function fakeIndexedDb() {
    const requests: Partial<IDBOpenDBRequest>[] = [];
    const factory = {
      open: vi.fn(() => {
        const request: Partial<IDBOpenDBRequest> = {};
        requests.push(request);
        return request as IDBOpenDBRequest;
      }),
    } as unknown as IDBFactory;
    return { factory, requests };
  }

  /** A database with one record, the way the store reads and writes it. */
  function fakeDatabase(record: unknown) {
    const close = vi.fn();
    const db = {
      close,
      transaction: () => {
        const transaction: { oncomplete?: () => void; objectStore: () => unknown } = {
          objectStore: () => ({
            get: () => {
              const request: { result?: unknown; onsuccess?: () => void } = { result: record };
              queueMicrotask(() => request.onsuccess?.());
              return request;
            },
            put: (value: unknown) => {
              record = value;
              queueMicrotask(() => transaction.oncomplete?.());
            },
          }),
        };
        return transaction;
      },
    } as unknown as IDBDatabase;
    return { db, close, current: () => record };
  }

  it("gives up on an open that never answers, and tries again the next time", async () => {
    vi.useFakeTimers();
    const { factory, requests } = fakeIndexedDb();
    const store = createCatalogStore(() => factory);

    const first = store.read();
    await vi.advanceTimersByTimeAsync(STORE_TIMEOUT_MS + 1);
    await expect(first).resolves.toBeUndefined();

    // A write while the database is silent is dropped instead of hanging.
    const write = store.write({ version: "3", emojis: [], categories: [] });
    await vi.advanceTimersByTimeAsync(STORE_TIMEOUT_MS + 1);
    await expect(write).resolves.toBeUndefined();
    expect(factory.open).toHaveBeenCalledTimes(2);

    // An open that answers after everyone stopped waiting is closed again.
    const late = fakeDatabase(undefined);
    Object.assign(requests[0] as object, { result: late.db });
    requests[0]?.onsuccess?.call(requests[0] as IDBOpenDBRequest, new Event("success"));
    expect(late.close).toHaveBeenCalled();
  });

  it("treats a blocked open as no stored copy", async () => {
    const { factory, requests } = fakeIndexedDb();
    const store = createCatalogStore(() => factory);
    const reading = store.read();
    requests[0]?.onblocked?.call(
      requests[0] as IDBOpenDBRequest,
      new Event("blocked") as IDBVersionChangeEvent,
    );
    await expect(reading).resolves.toBeUndefined();
  });

  it("works without IndexedDB at all", async () => {
    const store = createCatalogStore(() => undefined);
    await expect(store.read()).resolves.toBeUndefined();
    await expect(
      store.write({ version: "1", emojis: [], categories: [] }),
    ).resolves.toBeUndefined();
  });

  it("uses one connection for reads and writes, and lets go of it when another tab upgrades the database", async () => {
    const { factory, requests } = fakeIndexedDb();
    const store = createCatalogStore<string, string>(() => factory);
    const database = fakeDatabase({ version: "7", emojis: ["neko"], categories: [] });
    const answer = (index: number) => {
      Object.assign(requests[index] as object, { result: database.db });
      requests[index]?.onsuccess?.call(requests[index] as IDBOpenDBRequest, new Event("success"));
    };

    const reading = store.read();
    answer(0);
    await expect(reading).resolves.toEqual({ version: "7", emojis: ["neko"], categories: [] });
    await store.write({ version: "8", emojis: ["neko", "inu"], categories: [] });
    await expect(store.read()).resolves.toEqual({
      version: "8",
      emojis: ["neko", "inu"],
      categories: [],
    });
    expect(factory.open).toHaveBeenCalledTimes(1);

    database.db.onversionchange?.call(
      database.db,
      new Event("versionchange") as IDBVersionChangeEvent,
    );
    expect(database.close).toHaveBeenCalled();
    const again = store.read();
    expect(factory.open).toHaveBeenCalledTimes(2);
    answer(1);
    await expect(again).resolves.toEqual({ version: "8", emojis: ["neko", "inu"], categories: [] });
  });
});
