import i18n from "#/lib/i18n";

/** A position or length as a clock: `0:07`, `12:34`, `1:02:03`. */
export function formatClock(seconds: number): string {
  const total = Number.isFinite(seconds) ? Math.max(0, Math.floor(seconds)) : 0;
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

/**
 * Reads a clock typed by a person (`1:02:03`, `12:34`, `75`) into seconds;
 * `null` when it is not one.
 */
export function parseClock(text: string): number | null {
  const parts = text.trim().split(":");
  if (parts.length === 0 || parts.length > 3 || parts.some((part) => !/^\d{1,5}(\.\d+)?$/.test(part))) return null;
  const numbers = parts.map(Number);
  // Only the leading part may run past 59 ("75" is 1:15, "1:75" is a typo).
  if (numbers.slice(1).some((value) => value >= 60)) return null;
  return numbers.reduce((total, value) => total * 60 + value, 0);
}

/**
 * The `?t=` of a deep link in seconds: plain seconds (`90`), a clock (`1:30`)
 * or the `1h2m3s` form other video sites use. `null` when unreadable.
 */
export function parseTimeParam(value: string | undefined): number | null {
  if (!value) return null;
  const text = value.trim().toLowerCase();
  if (/^\d+(\.\d+)?s?$/.test(text)) return Number.parseFloat(text);
  if (text.includes(":")) return parseClock(text);
  const match = /^(?:(\d+)h)?(?:(\d+)m)?(?:(\d+)s)?$/.exec(text);
  if (!match?.[0]) return null;
  return Number(match[1] ?? 0) * 3600 + Number(match[2] ?? 0) * 60 + Number(match[3] ?? 0);
}

/** A count for cards: `1,234` stays exact, bigger numbers go compact (`1.2万` / `12K`). */
export function formatCount(value: number | bigint): string {
  const count = Number(value);
  if (count < 10_000) return count.toLocaleString(i18n.language);
  return new Intl.NumberFormat(i18n.language, { notation: "compact", maximumFractionDigits: 1 }).format(count);
}

/** How long ago something was published, coarsely: `3時間前`, `2週間前`, `1年前`. */
export function formatAge(fromMs: number, nowMs = Date.now()): string {
  const minutes = Math.floor(Math.max(0, nowMs - fromMs) / 60_000);
  const ago = (unit: string, count?: number) => i18n.t(`age.${unit}`, { ns: "video", count });
  if (minutes < 1) return ago("now");
  if (minutes < 60) return ago("minutes", minutes);
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return ago("hours", hours);
  const days = Math.floor(hours / 24);
  if (days < 7) return ago("days", days);
  if (days < 30) return ago("weeks", Math.floor(days / 7));
  if (days < 365) return ago("months", Math.floor(days / 30));
  return ago("years", Math.floor(days / 365));
}

/** Watch time for analytics: `42秒`, `12分`, `3.4時間`. */
export function formatWatchTime(seconds: number): string {
  const span = (unit: string, value: string) => i18n.t(`watchTime.${unit}`, { ns: "video", value });
  if (seconds < 60) return span("seconds", String(Math.round(seconds)));
  if (seconds < 3600) return span("minutes", String(Math.round(seconds / 60)));
  const hours = seconds / 3600;
  return span("hours", hours < 100 ? hours.toFixed(1) : Math.round(hours).toLocaleString(i18n.language));
}

/** `YYYY-MM-DD` of a local calendar day, as the analytics API takes dates. */
export function isoDay(date: Date): string {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

/** The value a `<input type="datetime-local">` shows for a moment in local time. */
export function toLocalInput(date: Date): string {
  return `${isoDay(date)}T${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

/**
 * The video ID in what a person pasted: a bare ID, or a watch link of this
 * app (`…/watch/<id>?t=…`). `null` when it holds neither.
 */
export function videoIdFromInput(input: string): string | null {
  const text = input.trim();
  if (!text) return null;
  const fromPath = /\/watch\/([A-Za-z0-9_-]{6,64})/.exec(text);
  if (fromPath) return fromPath[1] ?? null;
  return /^[A-Za-z0-9_-]{6,64}$/.test(text) ? text : null;
}
