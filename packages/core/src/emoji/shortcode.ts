/** How an emoji is written in text, decks and `RecordUse`. */
export const shortcode = (name: string) => `:${name}:`;

/** The name inside a `:shortcode:`, or `undefined` for anything else (a Unicode emoji). */
export function shortcodeName(text: string): string | undefined {
  return /^:[a-z0-9_]{2,32}:$/i.test(text) ? text.slice(1, -1).toLowerCase() : undefined;
}

/**
 * The page of an emoji. The server links to `/emoji/<name>`; the one name
 * that is also a page of this area is written with its colons, which
 * `GetEmoji` accepts.
 */
export function emojiPageParam(name: string): string {
  return name === "submissions" ? shortcode(name) : name;
}
