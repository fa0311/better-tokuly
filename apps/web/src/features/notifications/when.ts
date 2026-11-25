import { formatWhen as formatWhenIn } from "@tokuly/core/notifications";
import { useEffect, useState } from "react";
import i18n from "#/lib/i18n";

export {
  type DayBucket,
  dayBucket,
  minutesToTime,
  timeToMinutes,
} from "@tokuly/core/notifications";

/** "3分前" for the last week, then the date ("11月3日", with the year once it differs). */
export function formatWhen(atMs: number, nowMs: number, language: string): string {
  return formatWhenIn(i18n, atMs, nowMs, language);
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
