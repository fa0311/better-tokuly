import type { TFunction } from "i18next";

export function lateLabel(t: TFunction<"moments">, seconds: number): string {
  if (seconds >= 86_400) return t("friends.lateDays", { count: Math.floor(seconds / 86_400) });
  if (seconds >= 3_600) return t("friends.lateHours", { count: Math.floor(seconds / 3_600) });
  return t("friends.lateMinutes", { count: Math.max(1, Math.ceil(seconds / 60)) });
}
