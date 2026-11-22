/** A piece of text as `<EmojiText>` draws it. */
export type TextSegment = { kind: "text"; text: string } | { kind: "emoji"; name: string; raw: string };

const SHORTCODE = /:([A-Za-z0-9_]{2,32}):/g;
const WORD_CHARACTER = /[A-Za-z0-9]/;

/**
 * Splits text at the `:shortcode:`s that `known` recognises. Everything else
 * stays text, including codes of emoji that do not exist, Unicode emoji and
 * anything that looks like markup. Like on Misskey, a code glued to a letter
 * or digit is no code: the `:30:` of `12:30:45` is a time, not an emoji.
 */
export function splitEmojiText(text: string, known: (name: string) => boolean): TextSegment[] {
  const segments: TextSegment[] = [];
  let plainFrom = 0;
  SHORTCODE.lastIndex = 0;
  for (let match = SHORTCODE.exec(text); match; match = SHORTCODE.exec(text)) {
    const start = match.index;
    const end = start + match[0].length;
    const name = (match[1] as string).toLowerCase();
    const glued = (start > 0 && WORD_CHARACTER.test(text[start - 1] as string)) || (end < text.length && WORD_CHARACTER.test(text[end] as string));
    if (glued || !known(name)) {
      // The closing colon may open the next code (`:nope:neko:`).
      SHORTCODE.lastIndex = end - 1;
      continue;
    }
    if (start > plainFrom) segments.push({ kind: "text", text: text.slice(plainFrom, start) });
    segments.push({ kind: "emoji", name, raw: match[0] });
    plainFrom = end;
  }
  if (plainFrom < text.length) segments.push({ kind: "text", text: text.slice(plainFrom) });
  return segments;
}

/** The custom emoji names written in a text, each once, in order of appearance (for `recordEmojiUse`). */
export function shortcodesIn(text: string, known: (name: string) => boolean): string[] {
  const names = new Set<string>();
  for (const segment of splitEmojiText(text, known)) if (segment.kind === "emoji") names.add(segment.name);
  return [...names];
}

/**
 * The `:prefix` being typed right before the caret, for autocomplete in
 * composers: the characters after a colon that starts a word.
 * `undefined` when the caret is not in one.
 */
export function shortcodePrefixAt(text: string, caret: number): { prefix: string; start: number } | undefined {
  const match = /(?:^|[^A-Za-z0-9:]):([^\s:]{1,32})$/u.exec(text.slice(0, caret));
  if (!match) return undefined;
  const prefix = match[1] as string;
  return { prefix, start: caret - prefix.length - 1 };
}
