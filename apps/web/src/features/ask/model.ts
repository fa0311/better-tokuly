import type { common } from "@tokuly/api";
import * as shared from "@tokuly/core/ask";

/** Limits of the API, for counters and `maxLength` (the question's and the answer's are `@tokuly/core/ask`). */
export const BOX_PROMPT_MAX = 80;
export const PROMPT_MAX = 100;
export const MUTED_WORD_MAX = 30;
export const MUTED_WORDS_MAX = 100;

/** A text the server sends in both languages, in the one the app is shown in. */
export function localized(text: common.LocalizedText | undefined, language: string): string {
  if (!text) return "";
  return (language.startsWith("ja") ? text.ja || text.en : text.en || text.ja) ?? "";
}

/** The public page of a box: what a share link and the image card point at. */
export function boxShareUrl(handle: string, origin: string = window.location.origin): string {
  return shared.boxShareUrl(handle, origin);
}

/** The public page of a box with one answer brought to the top. */
export function answerShareUrl(
  handle: string,
  questionId: string,
  origin: string = window.location.origin,
): string {
  return shared.answerShareUrl(handle, questionId, origin);
}

/**
 * Whether something sent to a person goes out under the sender's name.
 * Anonymous by default; signed when the sender chooses it, or when it was the
 * only way in from the start. A recipient who stops hiding senders while one
 * is writing (read again after they lost a friend) signs nothing by itself:
 * the sender keeps the choice, and sending waits (`allowed`) until they sign.
 */
export function signature(input: {
  allowSigned: boolean;
  anonymousAvailable: boolean;
  /** Whether anonymity was available when the sender began writing. */
  beganAnonymous: boolean;
  wantSigned: boolean;
}): { choice: boolean; signed: boolean; allowed: boolean } {
  const { allowSigned, anonymousAvailable, beganAnonymous, wantSigned } = input;
  const choice = allowSigned && (anonymousAvailable || beganAnonymous);
  const signed = allowSigned && (wantSigned || !choice);
  return { choice, signed, allowed: signed || anonymousAvailable };
}

/** A countdown as a clock: "59:07", or "1:02:03" from an hour up. Never negative. */
export function countdownClock(ms: number): string {
  const total = Math.max(0, Math.ceil(ms / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  const two = (value: number) => String(value).padStart(2, "0");
  return hours > 0 ? `${hours}:${two(minutes)}:${two(seconds)}` : `${two(minutes)}:${two(seconds)}`;
}

/**
 * Splits the muted-word field into words: separated by commas (either
 * width), Japanese commas or line breaks; trimmed, without empties and
 * repeats.
 */
export function parseMutedWords(input: string): string[] {
  const seen = new Set<string>();
  for (const piece of input.split(/[,、，\n]/)) {
    const word = piece.trim();
    if (word) seen.add(word);
  }
  return [...seen];
}

/**
 * Breaks a text into lines no wider than `maxWidth`, measuring with
 * `measure`. Latin words stay whole where they fit; everything else (Japanese
 * has no spaces) breaks between characters. At most `maxLines` lines: the
 * last one ends in an ellipsis when text is left over.
 */
export function wrapText(
  text: string,
  maxWidth: number,
  maxLines: number,
  measure: (text: string) => number,
): string[] {
  const lines: string[] = [];
  const pieces = text.replace(/\r/g, "").split("\n");
  let truncated = false;

  outer: for (const paragraph of pieces) {
    // Tokens: runs of Latin letters/digits (kept together), or single characters.
    const tokens = paragraph.match(/[A-Za-z0-9@#_'’.\-/:]+|\s+|[\s\S]/gu) ?? [];
    let line = "";
    for (const token of tokens) {
      if (measure(line + token) <= maxWidth) {
        line += token;
        continue;
      }
      // A token wider than a whole line is cut by characters.
      const units = measure(token) > maxWidth ? Array.from(token) : [token];
      for (const unit of units) {
        if (measure(line + unit) <= maxWidth || line === "") {
          line += unit;
          continue;
        }
        lines.push(line.trimEnd());
        if (lines.length === maxLines) {
          truncated = true;
          break outer;
        }
        line = unit.trimStart();
      }
    }
    lines.push(line.trimEnd());
    if (lines.length === maxLines) {
      truncated = paragraph !== pieces[pieces.length - 1];
      break;
    }
  }

  if (truncated && lines.length > 0) {
    let last = lines[lines.length - 1]!;
    while (last.length > 0 && measure(`${last}…`) > maxWidth)
      last = Array.from(last).slice(0, -1).join("");
    lines[lines.length - 1] = `${last}…`;
  }
  return lines;
}
