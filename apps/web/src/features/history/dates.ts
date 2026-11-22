import { accountTimeZone, shiftDate, zonedMoment } from "#/lib/account-time";

/**
 * Calendar days of the history, as `YYYY-MM-DD` strings on the account's
 * calendar (its time zone — the same zone the requests name, so "today"
 * means the same on the screen and on the server, wherever the device is).
 */

const DATE_PATTERN = /^(\d{4})-(\d{2})-(\d{2})$/;

/** Midnight at the start of a `YYYY-MM-DD` day, or `null` for anything that is not a real date. */
export function parseDay(date: string): Date | null {
  const match = DATE_PATTERN.exec(date);
  if (!match) return null;
  const [year, month, day] = [Number(match[1]), Number(match[2]), Number(match[3])];
  const parsed = new Date(year, month - 1, day);
  return parsed.getFullYear() === year &&
    parsed.getMonth() === month - 1 &&
    parsed.getDate() === day
    ? parsed
    : null;
}

export function formatDay(date: Date): string {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

/** The day `days` after (or before) `date`. */
export function shiftDay(date: string, days: number): string {
  const parsed = parseDay(date);
  if (!parsed) return date;
  parsed.setDate(parsed.getDate() + days);
  return formatDay(parsed);
}

/** The moments a day begins and ends (the start of the next day) in `zone`: the account's unless given. */
export function dayRange(
  date: string,
  zone: string = accountTimeZone(),
): { from: Date; to: Date } | null {
  if (!parseDay(date)) return null;
  return {
    from: new Date(zonedMoment(date, 0, zone)),
    to: new Date(zonedMoment(shiftDate(date, 1), 0, zone)),
  };
}

/** The month a `YYYY-MM-DD` day is in. */
export const monthOfDay = (date: string): Month => ({
  year: Number(date.slice(0, 4)),
  month: Number(date.slice(5, 7)),
});

export interface Month {
  year: number;
  /** 1-12. */
  month: number;
}

export function shiftMonth({ year, month }: Month, by: number): Month {
  const index = year * 12 + (month - 1) + by;
  return { year: Math.floor(index / 12), month: (index % 12) + 1 };
}

export const monthOf = (date: Date): Month => ({
  year: date.getFullYear(),
  month: date.getMonth() + 1,
});

/**
 * The days of a month laid out in weeks starting on Sunday; `null` pads
 * the first and last week.
 */
export function monthGrid({ year, month }: Month): (string | null)[][] {
  const first = new Date(year, month - 1, 1);
  const days = new Date(year, month, 0).getDate();
  const cells: (string | null)[] = Array.from({ length: first.getDay() }, () => null);
  for (let day = 1; day <= days; day++) cells.push(formatDay(new Date(year, month - 1, day)));
  while (cells.length % 7 !== 0) cells.push(null);
  const weeks: (string | null)[][] = [];
  for (let index = 0; index < cells.length; index += 7) weeks.push(cells.slice(index, index + 7));
  return weeks;
}
