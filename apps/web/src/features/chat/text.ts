import i18n from "#/lib/i18n";

/** Text and time formatting for chat. Pure apart from the current language. */

// ───────────────────────────── Message text ─────────────────────────────

export type Token =
  | { type: "text"; text: string }
  | { type: "link"; text: string; href: string }
  /** `@handle` of a member the server recognised. */
  | { type: "mention"; text: string; handle: string }
  /** `@all`. */
  | { type: "everyone"; text: string };

// A link runs as far as characters that can be in a URL go: Japanese text right behind it
// ("https://…を見て") is not part of it.
const URL_OR_MENTION = /(https?:\/\/[A-Za-z0-9\-._~:/?#[\]@!$&'()*+,;=%]+)|@([A-Za-z0-9_-]+)/g;
/** Punctuation that ends a sentence rather than a link. */
const TRAILING = /[.,!?;:'")\]]+$/;

/**
 * Splits message text into plain text, links and mentions. Only handles the
 * server reported as mentions are marked (`@someone` written about a
 * stranger stays text), and `@all` only when it reached everyone.
 */
export function tokenize(
  text: string,
  mentionedHandles: string[],
  mentionsEveryone: boolean,
): Token[] {
  const handles = new Set(mentionedHandles.map((handle) => handle.toLowerCase()));
  const tokens: Token[] = [];
  let position = 0;
  const pushText = (value: string) => {
    if (!value) return;
    const last = tokens.at(-1);
    if (last?.type === "text") last.text += value;
    else tokens.push({ type: "text", text: value });
  };

  for (const match of text.matchAll(URL_OR_MENTION)) {
    const start = match.index;
    pushText(text.slice(position, start));
    position = start + match[0].length;
    if (match[1]) {
      // A closing bracket belongs to the link when the link opened it ("…/wiki/Tokyo_(city)").
      let trailing = match[1].match(TRAILING)?.[0] ?? "";
      while (
        trailing.startsWith(")") &&
        match[1].slice(0, match[1].length - trailing.length).includes("(")
      )
        trailing = trailing.slice(1);
      const href = trailing ? match[1].slice(0, -trailing.length) : match[1];
      tokens.push({ type: "link", text: href, href });
      pushText(trailing);
    } else {
      const handle = match[2]!;
      // A mention starts a word: "mail@example" is not one.
      const startsWord = start === 0 || /[\s(（「、。]/.test(text[start - 1]!);
      if (startsWord && mentionsEveryone && handle.toLowerCase() === "all")
        tokens.push({ type: "everyone", text: match[0] });
      else if (startsWord && handles.has(handle.toLowerCase()))
        tokens.push({ type: "mention", text: match[0], handle });
      else pushText(match[0]);
    }
  }
  pushText(text.slice(position));
  return tokens;
}

/**
 * The `@…` being typed at the caret, for the mention suggestions: the text
 * after the `@` and where the `@` is. `null` when the caret is not in one.
 */
export function mentionAt(text: string, caret: number): { query: string; start: number } | null {
  const before = text.slice(0, caret);
  const match = before.match(/(^|[\s(（「、。])@([A-Za-z0-9_-]*)$/);
  if (!match) return null;
  return { query: match[2]!, start: caret - match[2]!.length - 1 };
}

// ───────────────────────────── Times ─────────────────────────────

const DAY_MS = 86_400_000;

function startOfDay(ms: number): number {
  const date = new Date(ms);
  date.setHours(0, 0, 0, 0);
  return date.getTime();
}

/** Calendar days between two moments (0 = same day). */
function daysBetween(fromMs: number, toMs: number): number {
  return Math.round((startOfDay(toMs) - startOfDay(fromMs)) / DAY_MS);
}

/** "14:05" */
export function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString(i18n.language, { hour: "2-digit", minute: "2-digit" });
}

/** The day separator in a conversation: 今日 / 昨日 / 11月3日(火) / 2025年11月3日(月). */
export function dayLabel(ms: number, nowMs = Date.now()): string {
  const days = daysBetween(ms, nowMs);
  if (days === 0) return i18n.t("time.today", { ns: "chat" });
  if (days === 1) return i18n.t("time.yesterday", { ns: "chat" });
  const sameYear = new Date(ms).getFullYear() === new Date(nowMs).getFullYear();
  return new Date(ms).toLocaleDateString(i18n.language, {
    year: sameYear ? undefined : "numeric",
    month: "long",
    day: "numeric",
    weekday: "short",
  });
}

/** When a conversation was last active, as short as the list needs: 14:05 / 昨日 / 火曜日 / 11/3 / 2025/11/3. */
export function listTime(ms: number, nowMs = Date.now()): string {
  if (!ms) return "";
  const days = daysBetween(ms, nowMs);
  if (days <= 0) return clock(ms);
  if (days === 1) return i18n.t("time.yesterday", { ns: "chat" });
  if (days < 7) return new Date(ms).toLocaleDateString(i18n.language, { weekday: "long" });
  const sameYear = new Date(ms).getFullYear() === new Date(nowMs).getFullYear();
  return new Date(ms).toLocaleDateString(i18n.language, {
    year: sameYear ? undefined : "numeric",
    month: "numeric",
    day: "numeric",
  });
}

/** A full date and time, for details ("read at", call history). */
export function dateTime(ms: number): string {
  return new Date(ms).toLocaleString(i18n.language, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** A length of time as a player shows it: 0:07, 3:25, 1:02:03. */
export function playTime(seconds: number): string {
  const total = Math.max(0, Math.round(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const rest = String(total % 60).padStart(2, "0");
  return hours > 0 ? `${hours}:${String(minutes).padStart(2, "0")}:${rest}` : `${minutes}:${rest}`;
}

/** A file size for humans: 820 B, 12 KB, 3.4 MB, 1.2 GB. */
export function fileSize(bytes: number): string {
  if (bytes < 1000) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1000;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit++;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}
