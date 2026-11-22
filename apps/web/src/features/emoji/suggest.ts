import { useCatalog } from "./catalog";
import { type EmojiChoice, searchEmoji } from "./search";
import { useUnicodeTable } from "./use";

/**
 * Emoji whose name or alias matches what is being typed after a colon, for
 * autocomplete in composers. `prefix` is the text after the colon (see
 * `shortcodePrefixAt(text, caret)`); a single Latin letter suggests nothing
 * yet (one kana or kanji is a word already). Each suggestion's `emoji` is what to insert: `:name:` for a custom
 * emoji, the character for a Unicode one.
 *
 *   const at = shortcodePrefixAt(text, caret);
 *   const suggestions = useEmojiSuggest(at?.prefix ?? "");
 *   // on pick: text.slice(0, at.start) + suggestion.emoji + " " + text.slice(caret)
 */
export function useEmojiSuggest(prefix: string, limit = 8): EmojiChoice[] {
  const wanted = prefix.trim();
  const active = Array.from(wanted).length >= 2 || /[\u0080-\uffff]/.test(wanted);
  const catalog = useCatalog();
  const unicode = useUnicodeTable(active);
  if (!active) return [];
  return searchEmoji(wanted, catalog.emojis, unicode?.all ?? [], limit);
}
