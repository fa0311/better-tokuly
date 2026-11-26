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

/** The calendar day `days` after (or before) a `YYYY-MM-DD` date. */
export function shiftDate(date: string, days: number): string {
  const [year = 1970, month = 1, day = 1] = date.split("-").map(Number);
  return new Date(Date.UTC(year, month - 1, day + days)).toISOString().slice(0, 10);
}
