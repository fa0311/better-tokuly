import { describe, expect, it } from "vitest";
import { applyChanges, type CatalogEmoji } from "./catalog";
import { normalizeQuery, searchEmoji } from "./search";
import { shortcodePrefixAt, shortcodesIn, splitEmojiText } from "./text";

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

const known =
  (...names: string[]) =>
  (name: string) =>
    names.includes(name);

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
    expect(splitEmojiText(":Neko:", known("neko"))).toEqual([
      { kind: "emoji", name: "neko", raw: ":Neko:" },
    ]);
  });

  it("does not read a time or a glued code as an emoji", () => {
    expect(splitEmojiText("12:30:45", known("30"))).toEqual([{ kind: "text", text: "12:30:45" }]);
    expect(splitEmojiText("abc:neko:", known("neko"))).toEqual([
      { kind: "text", text: "abc:neko:" },
    ]);
    expect(splitEmojiText("ねこ:neko:だよ", known("neko"))).toHaveLength(3);
  });

  it("finds codes that follow each other, also after an unknown one", () => {
    expect(
      splitEmojiText(":neko::inu:", known("neko", "inu")).map((segment) => segment.kind),
    ).toEqual(["emoji", "emoji"]);
    expect(splitEmojiText(":あ:neko:", known("neko"))).toEqual([
      { kind: "text", text: ":あ" },
      { kind: "emoji", name: "neko", raw: ":neko:" },
    ]);
  });

  it("lists each emoji of a text once", () => {
    expect(shortcodesIn(":neko: :inu: :neko: :nope:", known("neko", "inu"))).toEqual([
      "neko",
      "inu",
    ]);
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
  const customs = [
    custom("neko", { aliases: ["ねこ", "cat"] }),
    custom("neko_love", { aliases: ["猫"] }),
    custom("inu", { aliases: ["いぬ", "catdog"] }),
  ];

  it("normalizes case, width and kana", () => {
    expect(normalizeQuery("ＮＥＫＯ")).toBe("neko");
    expect(normalizeQuery("ネコ")).toBe("ねこ");
    expect(normalizeQuery("ﾈｺ")).toBe("ねこ");
    expect(normalizeQuery(":neko:")).toBe("neko");
  });

  it("finds custom emoji by name and by a Japanese alias", () => {
    expect(searchEmoji("neko", customs, []).map((choice) => choice.emoji)).toEqual([
      ":neko:",
      ":neko_love:",
    ]);
    expect(searchEmoji("ネコ", customs, []).map((choice) => choice.emoji)).toEqual([":neko:"]);
  });
});

describe("applyChanges", () => {
  const current = {
    version: 3n,
    emojis: [custom("neko"), custom("inu")],
    categories: [{ name: "どうぶつ", emojiCount: 2 }],
  };

  it("replaces by ID, removes, and keeps the list ordered by name", () => {
    const next = applyChanges(current, {
      version: 5n,
      full: false,
      upserted: [{ ...custom("neko"), name: "nyan" }, custom("ahiru")],
      removedIds: ["id-inu"],
      categories: [],
    });
    expect(next.version).toBe(5n);
    expect(next.emojis.map((entry) => entry.name)).toEqual(["ahiru", "nyan"]);
    expect(next.byName.has("neko")).toBe(false);
    // A newer version always carries the categories: none are left.
    expect(next.categories).toEqual([]);
  });

  it("drops the copy when the answer is the whole catalogue", () => {
    const next = applyChanges(current, {
      version: 9n,
      full: true,
      upserted: [custom("tori")],
      removedIds: [],
      categories: [{ name: "とり", emojiCount: 1 }],
    });
    expect(next.emojis.map((entry) => entry.name)).toEqual(["tori"]);
    expect(next.categories).toEqual([{ name: "とり", emojiCount: 1 }]);
  });
});
