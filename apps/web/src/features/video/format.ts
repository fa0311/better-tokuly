import * as core from "@tokuly/core/video";
import i18n from "#/lib/i18n";

/** Clocks, counts and ages are `@tokuly/core/video`; the ones that speak get the web's language here. */
export { formatClock, parseClock, parseTimeParam, videoIdFromInput } from "@tokuly/core/video";

/** A count for cards: `1,234` stays exact, bigger numbers go compact (`1.2万` / `12K`). */
export const formatCount = (value: number | bigint): string =>
  core.formatCount(i18n.language, value);

/** How long ago something was published, coarsely: `3時間前`, `2週間前`, `1年前`. */
export const formatAge = (fromMs: number, nowMs = Date.now()): string =>
  core.formatAge(i18n, fromMs, nowMs);

/** Watch time for analytics: `42秒`, `12分`, `3.4時間`. */
export function formatWatchTime(seconds: number): string {
  const span = (unit: string, value: string) => i18n.t(`watchTime.${unit}`, { ns: "video", value });
  if (seconds < 60) return span("seconds", String(Math.round(seconds)));
  if (seconds < 3600) return span("minutes", String(Math.round(seconds / 60)));
  const hours = seconds / 3600;
  return span(
    "hours",
    hours < 100 ? hours.toFixed(1) : Math.round(hours).toLocaleString(i18n.language),
  );
}

/** `YYYY-MM-DD` of a local calendar day, as the analytics API takes dates. */
export function isoDay(date: Date): string {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

/** The value a `<input type="datetime-local">` shows for a moment in local time. */
export function toLocalInput(date: Date): string {
  return `${isoDay(date)}T${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}
