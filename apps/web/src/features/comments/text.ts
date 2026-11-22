/** A piece of a comment's text, as it is rendered. */
export type TextToken =
  | { kind: "text"; text: string }
  /** `@handle` of a user the server resolved. */
  | { kind: "mention"; text: string; handle: string }
  | { kind: "link"; text: string; url: string }
  /** A moment of the video written as a clock (`1:23`). */
  | { kind: "time"; text: string; seconds: number };

// A web address runs as far as characters that belong in one: Japanese text often follows it without a space.
const PATTERN =
  /(https?:\/\/[A-Za-z0-9\-._~:/?#[\]@!$&'()*+,;=%]+)|@([A-Za-z0-9_-]{3,24})|\b(\d{1,2}):(\d{2})(?::(\d{2}))?\b/g;
/** Punctuation that ends a sentence, not a link. */
const TRAILING = /[.,!?;:)\]']+$/;

/**
 * Splits a comment into what should be links: mentions of the users the
 * server resolved (`handles`, lowercase), web addresses, and — where the
 * subject is a video (`withTimes`) — clock times.
 */
export function tokenize(
  text: string,
  handles: ReadonlySet<string>,
  withTimes: boolean,
): TextToken[] {
  const tokens: TextToken[] = [];
  let last = 0;
  const push = (token: TextToken, start: number, end: number) => {
    if (start > last) tokens.push({ kind: "text", text: text.slice(last, start) });
    tokens.push(token);
    last = end;
  };
  for (const match of text.matchAll(PATTERN)) {
    const start = match.index;
    const whole = match[0];
    if (match[1]) {
      const url = whole.replace(TRAILING, "");
      push({ kind: "link", text: url, url }, start, start + url.length);
    } else if (match[2]) {
      // An address like `mail@example` is not a mention: the `@` must start a word.
      const before = start > 0 ? text[start - 1] : "";
      if (handles.has(match[2].toLowerCase()) && !/[A-Za-z0-9_.-]/.test(before ?? ""))
        push(
          { kind: "mention", text: whole, handle: match[2].toLowerCase() },
          start,
          start + whole.length,
        );
    } else if (withTimes && match[3] !== undefined && match[4] !== undefined) {
      const [first, second, third] = [
        Number(match[3]),
        Number(match[4]),
        match[5] === undefined ? undefined : Number(match[5]),
      ];
      if (second >= 60 || (third !== undefined && third >= 60)) continue;
      const seconds =
        third === undefined ? first * 60 + second : first * 3600 + second * 60 + third;
      push({ kind: "time", text: whole, seconds }, start, start + whole.length);
    }
  }
  if (last < text.length) tokens.push({ kind: "text", text: text.slice(last) });
  return tokens;
}

/**
 * The mention being typed right before the caret: where its `@` is and what
 * follows it so far. `null` when the caret is not inside one.
 */
export function mentionAtCaret(
  text: string,
  caret: number,
): { start: number; query: string } | null {
  const match = /(?:^|[^A-Za-z0-9_@.-])@([A-Za-z0-9_-]{0,24})$/.exec(text.slice(0, caret));
  if (!match) return null;
  const query = match[1] ?? "";
  return { start: caret - query.length - 1, query };
}

/** The text with the mention at `start` completed to `@handle ` and where the caret goes. */
export function completeMention(
  text: string,
  start: number,
  caret: number,
  handle: string,
): { text: string; caret: number } {
  const inserted = `@${handle} `;
  // Do not double the space when one already follows.
  const rest = text.slice(caret).replace(/^ /, "");
  return { text: text.slice(0, start) + inserted + rest, caret: start + inserted.length };
}
