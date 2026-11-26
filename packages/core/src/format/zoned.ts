/**
 * Dates and times on a zone's clock, wherever the device is: the API's
 * calendar dates (`YYYY-MM-DD`) and times of day are in the account's time
 * zone (`identity.Me.timeZone`), not the device's. Each app reads the
 * account's zone and passes it in.
 */

/** The zone accounts have until they change it (and the fallback for one the runtime does not know). */
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

/** What the clock in `zone` shows at `ms`, written as if it were UTC (to the minute). */
function wallClock(ms: number, zone: string): number {
  const shown = zonedTime(ms, zone);
  const [year = 1970, month = 1, day = 1] = shown.date.split("-").map(Number);
  return Date.UTC(year, month - 1, day, 0, shown.minute);
}

/**
 * The moment (epoch milliseconds) at which the clock in `zone` shows `date`
 * and `minute` after midnight. A time shown twice (the clocks went back) is
 * its first showing; a time a daylight-saving jump skips resolves to the
 * moment the jump happens.
 */
export function zonedMoment(date: string, minute: number, zone: string): number {
  const [year = 1970, month = 1, day = 1] = date.split("-").map(Number);
  const wanted = Date.UTC(year, month - 1, day, 0, minute);
  // The zone's offset a day either side: the moment is wanted minus one of them.
  const offsetAt = (ms: number) => wallClock(ms, zone) - Math.floor(ms / 60_000) * 60_000;
  const [earlier, later] = [
    wanted - offsetAt(wanted + 86_400_000),
    wanted - offsetAt(wanted - 86_400_000),
  ].sort((a, b) => a - b) as [number, number];
  for (const moment of [earlier, later]) if (wallClock(moment, zone) === wanted) return moment;
  // Skipped: the clock jumps somewhere between the two; find the first minute after it.
  let [low, high] = [earlier, later];
  const before = offsetAt(low);
  while (high - low > 60_000) {
    const middle = low + Math.floor((high - low) / 120_000) * 60_000;
    if (offsetAt(middle) === before) low = middle;
    else high = middle;
  }
  return high;
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

/** The calendar day `days` after (or before) a `YYYY-MM-DD` date. */
export function shiftDate(date: string, days: number): string {
  const [year = 1970, month = 1, day = 1] = date.split("-").map(Number);
  return new Date(Date.UTC(year, month - 1, day + days)).toISOString().slice(0, 10);
}
