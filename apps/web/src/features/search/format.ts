import { type Timestamp, toMillis } from "@tokuly/api";
import { compactAge } from "#/lib/format";
import i18n from "#/lib/i18n";

const WEEK_MS = 7 * 24 * 60 * 60 * 1000;

/** "3時間前" for the last week, a date ("11月3日", with the year once it differs) before that. */
export function ageLabel(timestamp: Timestamp | undefined, nowMs = Date.now()): string {
  const at = toMillis(timestamp);
  if (!at) return "";
  if (nowMs - at < WEEK_MS) return compactAge(at, nowMs);
  const date = new Date(at);
  const sameYear = date.getFullYear() === new Date(nowMs).getFullYear();
  return new Intl.DateTimeFormat(
    i18n.language,
    sameYear
      ? { month: "short", day: "numeric" }
      : { year: "numeric", month: "short", day: "numeric" },
  ).format(date);
}

/** A playing time: "0:42", "12:05", "1:02:03". */
export function formatDuration(totalSeconds: number): string {
  const seconds = Math.max(0, Math.floor(totalSeconds));
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const rest = String(seconds % 60).padStart(2, "0");
  return hours > 0 ? `${hours}:${String(minutes).padStart(2, "0")}:${rest}` : `${minutes}:${rest}`;
}

/** 1,284 → "1284", 12,900 → "1.3万" / "12.9K": counters next to results. */
export function formatCount(value: number | bigint): string {
  return new Intl.NumberFormat(i18n.language, {
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(value);
}
