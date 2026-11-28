import type { i18n as I18n } from "i18next";

/**
 * How late a moment was posted: 「時間内!」 within the window, otherwise
 * 「3時間遅れ」 (minutes under an hour, rounded up so a late moment never
 * reads as on time).
 */
export function latenessText(i18n: I18n, lateSeconds: number | bigint): string {
  const seconds = Number(lateSeconds);
  if (seconds <= 0) return i18n.t("moments:lateness.onTime");
  const minutes = Math.ceil(seconds / 60);
  if (minutes < 60) return i18n.t("moments:lateness.minutes", { count: minutes });
  return i18n.t("moments:lateness.hours", { count: Math.floor(minutes / 60) });
}

/** What is left of the window as `m:ss`, the second rounded up (`0:00` only once it is over). */
export function countdownText(leftMs: number): string {
  const seconds = Math.max(0, Math.ceil(leftMs / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}
