import { afterEach, describe, expect, it, vi } from "vitest";
import enStrings from "#/i18n/en/emoji";
import jaStrings from "#/i18n/ja/emoji";
import { applyChanges, type CatalogEmoji } from "./catalog";
import { createCatalogStore, STORE_TIMEOUT_MS } from "./catalog-store";
import { normalizeQuery, searchEmoji } from "./search";
import { shortcodePrefixAt, shortcodesIn, splitEmojiText } from "./text";
import { UNICODE_EMOJI, UNICODE_GROUPS } from "./unicode-data";

function custom(name: string, fields: Partial<CatalogEmoji> = {}): CatalogEmoji {
  return { id: `id-${name}`, name, url: `/${name}.webp`, stills: [], animated: false, category: "", aliases: [], sensitive: false, localOnly: false, addedAt: 0, ...fields };
}

const known = (...names: string[]) => (name: string) => names.includes(name);

describe("splitEmojiText", () => {
  it("turns known shortcodes into emoji and keeps the text around them", () => {
    expect(splitEmojiText("やっほー :neko: げんき？", known("neko"))).toEqual([
      { kind: "text", text: "やっほー " },
      { kind: "emoji", name: "neko", raw: ":neko:" },
      { kind: "text", text: " げんき？" },
    ]);
  });

  it("leaves unknown codes, Unicode emoji and markup as text", () => {
    const text = ":nope: 😀 <img src=x onerror=alert(1)> :neko:";
    expect(splitEmojiText(text, known("neko"))).toEqual([
      { kind: "text", text: ":nope: 😀 <img src=x onerror=alert(1)> " },
      { kind: "emoji", name: "neko", raw: ":neko:" },
    ]);
  });

  it("matches names without regard to case and keeps what was typed", () => {
    expect(splitEmojiText(":Neko:", known("neko"))).toEqual([{ kind: "emoji", name: "neko", raw: ":Neko:" }]);
  });

  it("does not read a time or a glued code as an emoji", () => {
    expect(splitEmojiText("12:30:45", known("30"))).toEqual([{ kind: "text", text: "12:30:45" }]);
    expect(splitEmojiText("abc:neko:", known("neko"))).toEqual([{ kind: "text", text: "abc:neko:" }]);
    expect(splitEmojiText("ねこ:neko:だよ", known("neko"))).toHaveLength(3);
  });

  it("finds codes that follow each other, also after an unknown one", () => {
    expect(splitEmojiText(":neko::inu:", known("neko", "inu")).map((segment) => segment.kind)).toEqual(["emoji", "emoji"]);
    expect(splitEmojiText(":あ:neko:", known("neko"))).toEqual([
      { kind: "text", text: ":あ" },
      { kind: "emoji", name: "neko", raw: ":neko:" },
    ]);
  });

  it("lists each emoji of a text once", () => {
    expect(shortcodesIn(":neko: :inu: :neko: :nope:", known("neko", "inu"))).toEqual(["neko", "inu"]);
  });
});

describe("shortcodePrefixAt", () => {
  it("finds the code being typed before the caret", () => {
    expect(shortcodePrefixAt("やっほー :ne", 9)).toEqual({ prefix: "ne", start: 6 });
    expect(shortcodePrefixAt(":ねこ", 3)).toEqual({ prefix: "ねこ", start: 0 });
  });

  it("ignores finished codes, times and plain text", () => {
    expect(shortcodePrefixAt(":neko: ", 7)).toBeUndefined();
    expect(shortcodePrefixAt("12:30", 5)).toBeUndefined();
    expect(shortcodePrefixAt("neko", 4)).toBeUndefined();
  });
});

describe("searchEmoji", () => {
  const customs = [custom("neko", { aliases: ["ねこ", "cat"] }), custom("neko_love", { aliases: ["猫"] }), custom("inu", { aliases: ["いぬ", "catdog"] })];

  it("normalizes case, width and kana", () => {
    expect(normalizeQuery("ＮＥＫＯ")).toBe("neko");
    expect(normalizeQuery("ネコ")).toBe("ねこ");
    expect(normalizeQuery("ﾈｺ")).toBe("ねこ");
    expect(normalizeQuery(":neko:")).toBe("neko");
  });

  it("finds custom emoji by name and by a Japanese alias", () => {
    expect(searchEmoji("neko", customs, []).map((choice) => choice.emoji)).toEqual([":neko:", ":neko_love:"]);
    expect(searchEmoji("ネコ", customs, []).map((choice) => choice.emoji)).toEqual([":neko:"]);
  });

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

describe("applyChanges", () => {
  const current = { version: 3n, emojis: [custom("neko"), custom("inu")], categories: [{ name: "どうぶつ", emojiCount: 2 }] };

  it("replaces by ID, removes, and keeps the list ordered by name", () => {
    const next = applyChanges(current, { version: 5n, full: false, upserted: [{ ...custom("neko"), name: "nyan" }, custom("ahiru")], removedIds: ["id-inu"], categories: [] });
    expect(next.version).toBe(5n);
    expect(next.emojis.map((entry) => entry.name)).toEqual(["ahiru", "nyan"]);
    expect(next.byName.has("neko")).toBe(false);
    // A newer version always carries the categories: none are left.
    expect(next.categories).toEqual([]);
  });

  it("drops the copy when the answer is the whole catalogue", () => {
    const next = applyChanges(current, { version: 9n, full: true, upserted: [custom("tori")], removedIds: [], categories: [{ name: "とり", emojiCount: 1 }] });
    expect(next.emojis.map((entry) => entry.name)).toEqual(["tori"]);
    expect(next.categories).toEqual([{ name: "とり", emojiCount: 1 }]);
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

describe("strings", () => {
  type Strings = { [key: string]: string | Strings };
  const keysOf = (strings: Strings, prefix = ""): string[] => Object.entries(strings).flatMap(([key, value]) => (typeof value === "string" ? [prefix + key] : keysOf(value, `${prefix}${key}.`)));

  it("has the same keys in Japanese and English", () => {
    expect(keysOf(jaStrings).sort()).toEqual(keysOf(enStrings).sort());
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
    requests[0]?.onblocked?.call(requests[0] as IDBOpenDBRequest, new Event("blocked") as IDBVersionChangeEvent);
    await expect(reading).resolves.toBeUndefined();
  });

  it("works without IndexedDB at all", async () => {
    const store = createCatalogStore(() => undefined);
    await expect(store.read()).resolves.toBeUndefined();
    await expect(store.write({ version: "1", emojis: [], categories: [] })).resolves.toBeUndefined();
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
    await expect(store.read()).resolves.toEqual({ version: "8", emojis: ["neko", "inu"], categories: [] });
    expect(factory.open).toHaveBeenCalledTimes(1);

    database.db.onversionchange?.call(database.db, new Event("versionchange") as IDBVersionChangeEvent);
    expect(database.close).toHaveBeenCalled();
    const again = store.read();
    expect(factory.open).toHaveBeenCalledTimes(2);
    answer(1);
    await expect(again).resolves.toEqual({ version: "8", emojis: ["neko", "inu"], categories: [] });
  });
});
