import { useSyncExternalStore } from "react";
import { compactAge } from "#/lib/format";

const WEEK_MS = 7 * 24 * 60 * 60 * 1000;

/** "3分前" for a week, then the date (with the year once it is not this one). */
export function noteAge(postedMs: number, language: string, nowMs = Date.now()): string {
  if (nowMs - postedMs < WEEK_MS) return compactAge(postedMs, nowMs);
  const posted = new Date(postedMs);
  const sameYear = posted.getFullYear() === new Date(nowMs).getFullYear();
  return posted.toLocaleDateString(
    language,
    sameYear
      ? { month: "short", day: "numeric" }
      : { year: "numeric", month: "short", day: "numeric" },
  );
}

/** The full date and time, for tooltips and the note page. */
export function noteDateTime(postedMs: number, language: string): string {
  return new Date(postedMs).toLocaleString(language, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

const TICK_MS = 30_000;
const listeners = new Set<() => void>();
let tick = 0;
let timer: ReturnType<typeof setInterval> | undefined;

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  timer ??= setInterval(() => {
    tick++;
    for (const notify of listeners) notify();
  }, TICK_MS);
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      clearInterval(timer);
      timer = undefined;
    }
  };
}

/** Re-renders the caller every half minute, so relative times ("3分前") stay true. One clock for every card. */
export function useClock(): number {
  return useSyncExternalStore(
    subscribe,
    () => tick,
    () => 0,
  );
}
