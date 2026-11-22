import { useEffect, useState } from "react";
import { compactAge } from "#/lib/format";

const DAY_MS = 86_400_000;

/** The section of the inbox an entry falls into, by the reader's calendar. */
export type DayBucket = "today" | "yesterday" | "week" | "earlier";

export function dayBucket(atMs: number, nowMs: number): DayBucket {
  const midnight = new Date(nowMs);
  midnight.setHours(0, 0, 0, 0);
  const today = midnight.getTime();
  if (atMs >= today) return "today";
  if (atMs >= today - DAY_MS) return "yesterday";
  if (atMs >= today - 6 * DAY_MS) return "week";
  return "earlier";
}

/** "3分前" for the last week, then the date ("11月3日", with the year once it differs). */
export function formatWhen(atMs: number, nowMs: number, language: string): string {
  if (nowMs - atMs < 7 * DAY_MS) return compactAge(atMs, nowMs);
  const date = new Date(atMs);
  const sameYear = date.getFullYear() === new Date(nowMs).getFullYear();
  return date.toLocaleDateString(
    language,
    sameYear
      ? { month: "short", day: "numeric" }
      : { year: "numeric", month: "short", day: "numeric" },
  );
}

/** The current time, re-rendering the caller every `everyMs` so relative times stay true. */
export function useNow(everyMs = 30_000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), everyMs);
    return () => clearInterval(timer);
  }, [everyMs]);
  return now;
}

/** Minutes after midnight ⇄ the "HH:MM" of a time input. */
export function minutesToTime(minutes: number): string {
  const clamped = ((Math.round(minutes) % 1440) + 1440) % 1440;
  return `${String(Math.floor(clamped / 60)).padStart(2, "0")}:${String(clamped % 60).padStart(2, "0")}`;
}

export function timeToMinutes(time: string): number | null {
  const match = /^(\d{1,2}):(\d{2})/.exec(time);
  if (!match) return null;
  const minutes = Number(match[1]) * 60 + Number(match[2]);
  return minutes >= 0 && minutes < 1440 ? minutes : null;
}
