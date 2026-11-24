/**
 * How a note's text is read: its length as people count it, and the
 * hashtags, `@mentions` and links inside it. This mirrors the server's
 * reading (`tokuly_notes::text`), so what the client links is exactly what
 * the server indexed: hashtags and mentions are only recognised outside of
 * links, a `#` glued to a word is part of that word, and Japanese text may
 * follow a link or a mention without a space.
 */

/** The longest note, in user-perceived characters. */
export const NOTE_MAX_CHARS = 3000;
const MAX_HASHTAG_CHARS = 50;

const segmenter =
  typeof Intl !== "undefined" && "Segmenter" in Intl
    ? new Intl.Segmenter(undefined, { granularity: "grapheme" })
    : null;

/** Length as people count it: an emoji family or a kana with dakuten is one character. */
export function charCount(text: string): number {
  if (!segmenter) return Array.from(text).length;
  let count = 0;
  for (const _ of segmenter.segment(text)) count++;
  return count;
}

export type Segment =
  | { type: "text"; text: string }
  | { type: "url"; text: string; url: string }
  /** `tag` is normalized: lowercase, no `#`. */
  | { type: "hashtag"; text: string; tag: string }
  /** `handle` is lowercase, without `@`. */
  | { type: "mention"; text: string; handle: string };

const WORD = /^[\p{Alphabetic}\p{N}_]$/u;
const ASCII_ALPHANUMERIC = /^[a-z0-9]$/i;
const HANDLE_CHAR = /^[a-z0-9_-]$/i;
const NUMERIC_OR_UNDERSCORE = /^[\p{N}_]+$/u;
const URL_FORBIDDEN = new Set(["<", ">", '"', "`", "{", "}", "|", "\\", "^"]);
const URL_TRAILING = new Set([".", ",", "!", "?", ":", ";", "'", "]"]);

const isWord = (char: string): boolean => WORD.test(char);

function runEnd(chars: string[], from: number, accept: (char: string) => boolean): number {
  let end = from;
  while (end < chars.length && accept(chars[end]!)) end++;
  return end;
}

/** If a link starts at `start`, the index just past it. */
function urlEnd(chars: string[], start: number, previous: string | undefined): number | undefined {
  if (previous !== undefined && ASCII_ALPHANUMERIC.test(previous)) return undefined;
  const schemeLength = ["https://", "http://"].find(
    (scheme) =>
      chars
        .slice(start, start + scheme.length)
        .join("")
        .toLowerCase() === scheme,
  )?.length;
  if (!schemeLength) return undefined;
  let end = runEnd(chars, start + schemeLength, (char) => {
    const code = char.codePointAt(0)!;
    return code >= 0x21 && code <= 0x7e && !URL_FORBIDDEN.has(char);
  });
  // Sentence punctuation right after a link is not part of it.
  while (end > start + schemeLength) {
    const last = chars[end - 1]!;
    const inside = chars.slice(start, end);
    const unbalancedClose =
      last === ")" &&
      inside.filter((char) => char === ")").length > inside.filter((char) => char === "(").length;
    if (URL_TRAILING.has(last) || unbalancedClose) end--;
    else break;
  }
  return end > start + schemeLength ? end : undefined;
}

/**
 * A hashtag as the server stores and compares it: without `#`, compatibility
 * forms folded and lowercased. `undefined` when it is not a usable hashtag
 * (empty, too long, digits only, or with anything but letters, digits, `_`).
 */
export function normalizeHashtag(input: string): string | undefined {
  const trimmed = input.trim().replace(/^[#＃]/, "");
  const tag = trimmed.normalize("NFKC").toLowerCase();
  const chars = Array.from(tag);
  const valid =
    chars.length > 0 &&
    charCount(tag) <= MAX_HASHTAG_CHARS &&
    chars.every(isWord) &&
    !NUMERIC_OR_UNDERSCORE.test(tag);
  return valid ? tag : undefined;
}

/** Splits a note's text into plain runs, links, hashtags and mentions, in order. */
export function parseNoteText(text: string): Segment[] {
  const chars = Array.from(text);
  const segments: Segment[] = [];
  let plain = "";
  const push = (segment: Segment) => {
    if (plain) segments.push({ type: "text", text: plain });
    plain = "";
    segments.push(segment);
  };

  let index = 0;
  while (index < chars.length) {
    const previous = index > 0 ? chars[index - 1] : undefined;
    const linkEnd = urlEnd(chars, index, previous);
    if (linkEnd !== undefined) {
      const url = chars.slice(index, linkEnd).join("");
      push({ type: "url", text: url, url });
      index = linkEnd;
      continue;
    }
    const char = chars[index]!;
    // A `#` glued to the end of a word (`a#b`, `C#`, `色#fff`) is part of that word.
    const startsHashtag =
      previous === undefined || (!isWord(previous) && previous !== "/" && previous !== "&");
    // An `@` glued to Latin text is an address; Japanese is written without spaces, so `友達の@taro` mentions someone.
    const startsMention =
      previous === undefined ||
      (!ASCII_ALPHANUMERIC.test(previous) && !["_", "/", "@", "＠"].includes(previous));
    if ((char === "#" || char === "＃") && startsHashtag) {
      const end = runEnd(chars, index + 1, isWord);
      const raw = chars.slice(index + 1, end).join("");
      const tag = normalizeHashtag(raw);
      if (tag) push({ type: "hashtag", text: char + raw, tag });
      else plain += char + raw;
      index = Math.max(end, index + 1);
      continue;
    }
    if ((char === "@" || char === "＠") && startsMention) {
      const end = runEnd(chars, index + 1, (candidate) => HANDLE_CHAR.test(candidate));
      const raw = chars.slice(index + 1, end).join("");
      const handle = raw.toLowerCase();
      // `@name@example.com` is an address on another service.
      const foreign = chars[end] === "@";
      const shaped =
        handle.length >= 3 && handle.length <= 24 && ASCII_ALPHANUMERIC.test(handle[0]!);
      if (shaped && !foreign) push({ type: "mention", text: char + raw, handle });
      else plain += char + raw;
      index = Math.max(end, index + 1);
      continue;
    }
    plain += char;
    index++;
  }
  if (plain) segments.push({ type: "text", text: plain });
  return segments;
}

/** What is being typed at the caret, when it is a hashtag or a mention: the completion popup's cue. */
export interface Completion {
  kind: "hashtag" | "mention";
  /** What was typed after `#` / `@` so far (may be empty right after the sign). */
  query: string;
  /** Where the token starts (the sign itself) and ends (the caret), as string indices. */
  start: number;
  end: number;
}

const HASHTAG_AT_CARET = /(?:^|[^\p{Alphabetic}\p{N}_/&])[#＃]([\p{Alphabetic}\p{N}_]{0,50})$/u;
const MENTION_AT_CARET = /(?:^|[^a-z0-9_/@＠])[@＠]([a-z0-9_-]{0,24})$/i;

/** Looks left from the caret for a `#tag` or `@handle` being typed. */
export function completionAt(text: string, caret: number): Completion | undefined {
  const before = text.slice(0, caret);
  const hashtag = HASHTAG_AT_CARET.exec(before);
  if (hashtag)
    return {
      kind: "hashtag",
      query: hashtag[1]!,
      start: caret - hashtag[1]!.length - 1,
      end: caret,
    };
  const mention = MENTION_AT_CARET.exec(before);
  if (mention)
    return {
      kind: "mention",
      query: mention[1]!,
      start: caret - mention[1]!.length - 1,
      end: caret,
    };
  return undefined;
}

/** The text with the token being typed replaced by `replacement` and a space, plus the caret's new place. */
export function applyCompletion(
  text: string,
  completion: Completion,
  replacement: string,
): { text: string; caret: number } {
  const inserted = `${replacement} `;
  const next =
    text.slice(0, completion.start) + inserted + text.slice(completion.end).replace(/^ /, "");
  return { text: next, caret: completion.start + inserted.length };
}
