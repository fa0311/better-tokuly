import {
  DEFAULT_TIME_ZONE,
  untilNextDay,
  type ZonedTime,
  zonedDate,
  zonedTime,
} from "@tokuly/core/format";
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

export {
  DEFAULT_TIME_ZONE,
  shiftDate,
  untilNextDay,
  type ZonedTime,
  zonedDate,
  zonedMoment,
  zonedTime,
} from "@tokuly/core/format";

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
