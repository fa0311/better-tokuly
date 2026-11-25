import * as format from "@tokuly/core/format";
import i18n from "#/lib/i18n";

/** Formatting from `@tokuly/core/format`, in the language the web app shows. */
export { distanceMeters, formatDistance, kmh } from "@tokuly/core/format";

/** "NOW" within 10 s like the original marker pill, then a compact age. */
export function compactAge(fromMs: number, nowMs = Date.now()): string {
  return format.compactAge(i18n, fromMs, nowMs);
}

/** A duration like "45分" / "3時間" / "2日" (ja) or "45m" / "3h" / "2d" (en). */
export function compactDuration(ms: number): string {
  return format.compactDuration(i18n.language, ms);
}
