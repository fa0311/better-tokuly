/**
 * Custom emoji logic every app shares: query keys and options, the
 * catalogue copy as plain data (built, updated, looked up), `:shortcode:`s
 * in text, and searching emoji by name in Japanese and English. Where the
 * catalogue is kept, the Unicode table, submissions and the admin screens
 * stay in each app.
 */
export {
  applyChanges,
  buildCatalog,
  type Catalog,
  type CatalogCategory,
  type CatalogEmoji,
  catalogUpdate,
  EMPTY_CATALOG,
  findEmoji,
  stillUrl,
  toCatalogEmoji,
} from "./catalog";
export { emojiKeys } from "./keys";
export { DECK_MAX, emojiQueries } from "./queries";
export {
  customChoice,
  type EmojiChoice,
  normalizeQuery,
  searchEmoji,
  type UnicodeEmoji,
  unicodeChoice,
} from "./search";
export { emojiPageParam, shortcode, shortcodeName } from "./shortcode";
export { shortcodePrefixAt, shortcodesIn, splitEmojiText, type TextSegment } from "./text";
