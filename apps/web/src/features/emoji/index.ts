/**
 * What the emoji feature shares with every other feature: custom emoji
 * (`:shortcode:` pictures, Misskey-style) in text, names and reactions.
 *
 *   <EmojiText text={message.text} />                 // any text with its :shortcode:s drawn as emoji
 *   <Emoji name="neko" />                              // one custom emoji by name
 *   <EmojiPicker trigger={<Button …/>} onPick={(emoji) => insert(emoji)} />   // Unicode + custom, deck first
 *
 *   // `:` autocomplete in a composer:
 *   const at = shortcodePrefixAt(text, caret);          // { prefix, start } while a `:name` is being typed
 *   const suggestions = useEmojiSuggest(at?.prefix ?? "");
 *   // on pick: text.slice(0, at.start) + suggestion.emoji + " " + text.slice(caret)
 *
 *   recordEmojiUse(shortcodesIn(text))                  // after sending: feeds "recently used" and "popular"
 *
 * An emoji is always written the way it appears in text: `:name:` for a
 * custom emoji, the character itself for a Unicode one. That is what
 * `onPick` hands over, what reactions store and what `EmojiText` reads.
 *
 * The catalogue (every approved emoji) is kept in this browser and updated
 * live; `lookupEmoji(name)` reads it synchronously outside React and
 * `useCatalog()` inside. Nothing needs to be loaded or mounted first.
 */
export { emojiPageParam, shortcode, shortcodeName } from "./api";
export { type Catalog, type CatalogCategory, type CatalogEmoji, lookupEmoji, useCatalog } from "./catalog";
export { Emoji, EmojiImage, type EmojiImageProps, type EmojiProps } from "./emoji";
export { EmojiText, type EmojiTextProps } from "./emoji-text";
export { EmojiPicker, type EmojiPickerProps } from "./picker";
export type { EmojiChoice } from "./search";
export { useEmojiSuggest } from "./suggest";
export { shortcodePrefixAt, splitEmojiText, type TextSegment } from "./text";
export { recordEmojiUse } from "./use";

import { getCatalog } from "./catalog";
import { shortcodesIn as shortcodesInText } from "./text";

/** The custom emoji written in a text (`:name:` each, once), for `recordEmojiUse` after sending it. */
export function shortcodesIn(text: string): string[] {
  const catalog = getCatalog();
  return shortcodesInText(text, (name) => catalog.byName.has(name)).map((name) => `:${name}:`);
}
