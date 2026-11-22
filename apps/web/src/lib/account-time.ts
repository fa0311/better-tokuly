import { useEffect, useState } from "react";
import { useMe, useSessionStore } from "#/stores/session";

/**
 * Time as the account sees it.
 *
 * The API's calendar dates (`YYYY-MM-DD`) and times of day are in the
 * account's time zone (`identity.Me.timeZone`, Asia/Tokyo unless changed),
 * not the device's: someone travelling keeps their school day, their streak
 * and their "today". Anything that says "today" or compares against a date
 * from the API reads it here, never from `new Date()`'s local fields.
 */

/** The zone accounts have until they change it (and the fallback for one the browser does not know). */
export const DEFAULT_TIME_ZONE = "Asia/Tokyo";

const formatters = new Map<string, Intl.DateTimeFormat>();

function formatter(zone: string): Intl.DateTimeFormat {
  let format = formatters.get(zone);
  if (!format) {
    const options = {
      hourCycle: "h23",
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    } as const;
    try {
      format = new Intl.DateTimeFormat("en-US", { ...options, timeZone: zone });
    } catch {
      format = new Intl.DateTimeFormat("en-US", { ...options, timeZone: DEFAULT_TIME_ZONE });
    }
    formatters.set(zone, format);
  }
  return format;
}

/** A moment as a zone's clock shows it. */
export interface ZonedTime {
  /** The calendar date, `YYYY-MM-DD`. */
  date: string;
  /** Minutes after the zone's midnight. */
  minute: number;
  second: number;
}

/** What the clock on the wall says in `zone` at the moment `ms` (epoch milliseconds). */
export function zonedTime(ms: number, zone: string): ZonedTime {
  const parts: Record<string, string> = {};
  for (const part of formatter(zone).formatToParts(new Date(ms))) parts[part.type] = part.value;
  return {
    date: `${parts.year}-${parts.month}-${parts.day}`,
    minute: Number(parts.hour) * 60 + Number(parts.minute),
    second: Number(parts.second),
  };
}

/** The calendar date in `zone` at the moment `ms`. */
export function zonedDate(ms: number, zone: string): string {
  return zonedTime(ms, zone).date;
}

/**
 * The moment (epoch milliseconds) at which the clock in `zone` shows `date`
 * and `minute` after midnight. A time skipped by a daylight-saving jump
 * resolves to the moment the jump happens.
 */
export function zonedMoment(date: string, minute: number, zone: string): number {
  const [year = 1970, month = 1, day = 1] = date.split("-").map(Number);
  const wanted = Date.UTC(year, month - 1, day, 0, minute);
  // The zone's offset at a first guess, then once more at the corrected moment (offsets change twice a year).
  let moment = wanted;
  for (let pass = 0; pass < 2; pass++) {
    const shown = zonedTime(moment, zone);
    const [shownYear = 1970, shownMonth = 1, shownDay = 1] = shown.date.split("-").map(Number);
    moment += wanted - Date.UTC(shownYear, shownMonth - 1, shownDay, 0, shown.minute);
  }
  return moment;
}

/** Milliseconds from `ms` until the date changes in `zone`. */
export function untilNextDay(ms: number, zone: string): number {
  const today = zonedDate(ms, zone);
  const [year = 1970, month = 1, day = 1] = today.split("-").map(Number);
  // The calendar day after today, whatever the month; its midnight in the zone.
  const next = new Date(Date.UTC(year, month - 1, day + 1));
  const tomorrow = `${next.getUTCFullYear()}-${String(next.getUTCMonth() + 1).padStart(2, "0")}-${String(next.getUTCDate()).padStart(2, "0")}`;
  return Math.max(0, zonedMoment(tomorrow, 0, zone) - ms);
}

/**
 * Calls `onChange` with today's date in `zone` now and again whenever it
 * changes: at the zone's midnight, and when the page is looked at again
 * after a night in the background (where timers are late or never fire).
 * Returns a function that stops watching.
 */
export function watchToday(zone: string, onChange: (date: string) => void): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let last: string | undefined;
  const check = () => {
    clearTimeout(timer);
    const now = Date.now();
    const today = zonedDate(now, zone);
    if (today !== last) {
      last = today;
      onChange(today);
    }
    // A moment past midnight, so the next look is safely inside the new day; timers cannot wait longer than 24.8 days.
    timer = setTimeout(check, Math.min(untilNextDay(now, zone) + 250, 2_000_000_000));
  };
  check();
  if (typeof document !== "undefined") {
    document.addEventListener("visibilitychange", check);
    window.addEventListener("focus", check);
  }
  return () => {
    clearTimeout(timer);
    if (typeof document !== "undefined") {
      document.removeEventListener("visibilitychange", check);
      window.removeEventListener("focus", check);
    }
  };
}

/**
 * The signed-in account's time zone, read once (outside React, or where a
 * date is only needed at the moment something happens). Components that show
 * "today" use the hooks below, which follow midnight.
 */
export function accountTimeZone(): string {
  const { state } = useSessionStore.getState();
  return (state.status === "signedIn" && state.me.timeZone) || DEFAULT_TIME_ZONE;
}

/** Today's date (`YYYY-MM-DD`) on the account's calendar, at the moment `ms`. */
export function accountToday(ms: number = Date.now()): string {
  return zonedDate(ms, accountTimeZone());
}

/** The calendar day `days` after (or before) a `YYYY-MM-DD` date. */
export function shiftDate(date: string, days: number): string {
  const [year = 1970, month = 1, day = 1] = date.split("-").map(Number);
  return new Date(Date.UTC(year, month - 1, day + days)).toISOString().slice(0, 10);
}

/** The signed-in account's time zone. */
export function useAccountTimeZone(): string {
  return useMe()?.timeZone || DEFAULT_TIME_ZONE;
}

/**
 * Today's date (`YYYY-MM-DD`) in the account's time zone. The component is
 * drawn again when the date changes, so a page left open overnight moves on
 * to the new day.
 */
export function useAccountToday(): string {
  const zone = useAccountTimeZone();
  const [today, setToday] = useState(() => ({ zone, date: zonedDate(Date.now(), zone) }));
  useEffect(() => watchToday(zone, (date) => setToday({ zone, date })), [zone]);
  // Right after the zone changed, the stored date is still the old zone's.
  return today.zone === zone ? today.date : zonedDate(Date.now(), zone);
}

/** The current moment with what the account's clock shows at it. */
export interface AccountNow extends ZonedTime {
  /** Epoch milliseconds, for comparing against timestamps. */
  ms: number;
}

/**
 * "Now" in the account's time zone, re-read every `intervalMs` (and when the
 * page is looked at again) while the caller is mounted.
 */
export function useAccountNow(intervalMs = 30_000): AccountNow {
  const zone = useAccountTimeZone();
  const [ms, setMs] = useState(() => Date.now());
  useEffect(() => {
    const update = () => setMs(Date.now());
    const interval = window.setInterval(update, intervalMs);
    document.addEventListener("visibilitychange", update);
    return () => {
      window.clearInterval(interval);
      document.removeEventListener("visibilitychange", update);
    };
  }, [intervalMs]);
  return { ms, ...zonedTime(ms, zone) };
}
