import type { TFunction } from "i18next";

export interface Day<T> {
  /** `YYYY-MM-DD` in the device's time zone. */
  key: string;
  date: Date;
  items: T[];
}

/** `YYYY-MM-DD` of a moment, in the device's time zone. */
export function dayKey(date: Date): string {
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** Splits a list that is already in time order into runs of the same calendar day, keeping the order. */
export function groupByDay<T>(items: T[], millisOf: (item: T) => number): Day<T>[] {
  const days: Day<T>[] = [];
  for (const item of items) {
    const date = new Date(millisOf(item));
    const key = dayKey(date);
    const last = days.at(-1);
    if (last?.key === key) last.items.push(item);
    else days.push({ key, date, items: [item] });
  }
  return days;
}

/** "今日" / "昨日" / "11月20日(金)" — with the year once it is not this one. */
export function dayLabel(
  date: Date,
  language: string,
  t: TFunction<"notes">,
  now = new Date(),
): string {
  const key = dayKey(date);
  if (key === dayKey(now)) return t("days.today");
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (key === dayKey(yesterday)) return t("days.yesterday");
  const sameYear = date.getFullYear() === now.getFullYear();
  return date.toLocaleDateString(
    language,
    sameYear
      ? { month: "long", day: "numeric", weekday: "short" }
      : { year: "numeric", month: "long", day: "numeric", weekday: "short" },
  );
}
