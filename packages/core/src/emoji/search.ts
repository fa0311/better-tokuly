import type { CatalogEmoji } from "./catalog";

/** A Unicode emoji with the words it is found by; the first English word is its name. */
export interface UnicodeEmoji {
  emoji: string;
  en: string[];
  ja: string[];
}

/**
 * Makes two spellings of a word equal: case, width (ｎｅｋｏ → neko, ﾈｺ → ネコ)
 * and the difference between katakana and hiragana (ネコ → ねこ), the way the
 * server's `SearchEmoji` matches.
 */
export function normalizeQuery(text: string): string {
  return text
    .normalize("NFKC")
    .toLowerCase()
    .replace(/[ァ-ヶ]/g, (kana) => String.fromCharCode(kana.charCodeAt(0) - 0x60))
    .replace(/^:+|:+$/g, "")
    .trim();
}

/** 0 = the word itself, 1 = starts with the query, 2 = contains it, -1 = no match. */
function rank(words: readonly string[], query: string): number {
  let best = -1;
  for (const raw of words) {
    const word = normalizeQuery(raw);
    const score = word === query ? 0 : word.startsWith(query) ? 1 : word.includes(query) ? 2 : -1;
    if (score === 0) return 0;
    if (score > 0 && (best < 0 || score < best)) best = score;
  }
  return best;
}

/** An emoji a picker or an autocomplete offers: custom or Unicode. */
export type EmojiChoice =
  | { kind: "custom" /** `:name:` */; emoji: string; name: string; custom: CatalogEmoji }
  | { kind: "unicode" /** The character itself. */; emoji: string; name: string };

export const customChoice = (custom: CatalogEmoji): EmojiChoice => ({
  kind: "custom",
  emoji: `:${custom.name}:`,
  name: custom.name,
  custom,
});
export const unicodeChoice = (entry: UnicodeEmoji): EmojiChoice => ({
  kind: "unicode",
  emoji: entry.emoji,
  name: entry.en[0] ?? entry.emoji,
});

/**
 * Finds emoji by name and alias, custom and Unicode together, in Japanese
 * and English: exact matches first, then words starting with the query, then
 * words containing it. Among equals custom emoji lead (they are what a
 * community made its own), then names before aliases.
 */
export function searchEmoji(
  query: string,
  customs: readonly CatalogEmoji[],
  unicode: readonly UnicodeEmoji[],
  limit = 60,
): EmojiChoice[] {
  const wanted = normalizeQuery(query);
  if (!wanted) return [];
  const hits: { choice: EmojiChoice; score: number }[] = [];
  for (const custom of customs) {
    const byName = rank([custom.name], wanted);
    const byAlias = rank(custom.aliases, wanted);
    if (byName < 0 && byAlias < 0) continue;
    // Name hits are worth half a step more than alias hits of the same kind.
    const score = Math.min(byName < 0 ? 99 : byName * 2, byAlias < 0 ? 99 : byAlias * 2 + 1);
    hits.push({ choice: customChoice(custom), score });
  }
  for (const entry of unicode) {
    if (entry.emoji === query.trim()) {
      hits.push({ choice: unicodeChoice(entry), score: 0 });
      continue;
    }
    const score = Math.min(
      ...[rank(entry.en, wanted), rank(entry.ja, wanted)]
        .filter((value) => value >= 0)
        .map((value) => value * 2 + 1.5),
      99,
    );
    if (score < 99) hits.push({ choice: unicodeChoice(entry), score });
  }
  // A stable sort keeps the catalogue's and the table's own order among equals.
  return hits
    .sort((a, b) => a.score - b.score)
    .slice(0, limit)
    .map((hit) => hit.choice);
}
