import { accountTimeZone, accountToday, zonedMoment, zonedTime } from "#/lib/account-time";

/**
 * Days and times for plans, all on the account's calendar and clock (its
 * time zone, `lib/account-time`), wherever the device is: "today", the day
 * and time of an instant, the instant of a day and a time, and the zone
 * named in requests agree with each other and with the server.
 *
 * A day without a time is a `YYYY-MM-DD` string (the API's `*_date` fields).
 * Arithmetic on days (`addDays`, `monthGrid`…) is plain calendar arithmetic
 * and involves no zone.
 */

const pad = (n: number) => String(n).padStart(2, "0");

/** The calendar fields of a `Date` used as a zone-free calendar day (see `parseYmd`). */
function calendarYmd(date: Date): string {
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** The time zone plans work in and name in requests: the account's. */
export function planTimeZone(): string {
  return accountTimeZone();
}

/** `YYYY-MM-DD` of an instant on the account's calendar. */
export function ymd(date: Date, zone: string = planTimeZone()): string {
  return zonedTime(date.getTime(), zone).date;
}

/** Today on the account's calendar (its time zone, not the device's): what the API's dates are compared with. */
export function todayYmd(now: Date = new Date()): string {
  return accountToday(now.getTime());
}

/**
 * A `YYYY-MM-DD` day as a calendar `Date` for arithmetic and formatting (its local fields are
 * the day; it is not an instant to send anywhere); `null` when the text is not a real day.
 */
export function parseYmd(day: string): Date | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(day);
  if (!match) return null;
  const [year, month, date] = [Number(match[1]), Number(match[2]), Number(match[3])];
  const parsed = new Date(year, month - 1, date);
  return parsed.getFullYear() === year &&
    parsed.getMonth() === month - 1 &&
    parsed.getDate() === date
    ? parsed
    : null;
}

export function addDays(day: string, count: number): string {
  const date = parseYmd(day);
  if (!date) return day;
  date.setDate(date.getDate() + count);
  return calendarYmd(date);
}

/** Whole days from `from` to `to` (negative when `to` is earlier). */
export function daysBetween(from: string, to: string): number {
  const [a, b] = [parseYmd(from), parseYmd(to)];
  if (!a || !b) return 0;
  // Noon to noon, so a daylight-saving hour never rounds a day away.
  return Math.round(
    (Date.UTC(b.getFullYear(), b.getMonth(), b.getDate()) -
      Date.UTC(a.getFullYear(), a.getMonth(), a.getDate())) /
      86_400_000,
  );
}

/** `YYYY-MM` of a day. */
export function monthOf(day: string): string {
  return day.slice(0, 7);
}

export function addMonths(month: string, count: number): string {
  const [year, index] = [Number(month.slice(0, 4)), Number(month.slice(5, 7)) - 1 + count];
  const date = new Date(year, index, 1);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}`;
}

/**
 * The weeks a month page shows, Sunday first: every cell is a day, the ones
 * before the 1st and after the last belong to the neighbouring months.
 */
export function monthGrid(month: string): string[][] {
  const first = parseYmd(`${month}-01`);
  if (!first) return [];
  let cursor = addDays(calendarYmd(first), -first.getDay());
  const weeks: string[][] = [];
  do {
    const week: string[] = [];
    for (let i = 0; i < 7; i++) {
      week.push(cursor);
      cursor = addDays(cursor, 1);
    }
    weeks.push(week);
  } while (monthOf(cursor) === month);
  return weeks;
}

/** `HH:MM` of an instant on the account's clock (the value of a time input). */
export function timeOf(date: Date, zone: string = planTimeZone()): string {
  const { minute } = zonedTime(date.getTime(), zone);
  return `${pad(Math.floor(minute / 60))}:${pad(minute % 60)}`;
}

/** A day and a `HH:MM` time on the account's clock as an instant; `null` when either is malformed. */
export function combine(day: string, time: string, zone: string = planTimeZone()): Date | null {
  const match = /^([01]\d|2[0-3]):([0-5]\d)$/.exec(time);
  if (!parseYmd(day) || !match) return null;
  return new Date(zonedMoment(day, Number(match[1]) * 60 + Number(match[2]), zone));
}

/** The value of a `datetime-local` input for an instant, on the account's clock. */
export function toDateTimeInput(date: Date, zone: string = planTimeZone()): string {
  return `${ymd(date, zone)}T${timeOf(date, zone)}`;
}

/** The instant a `datetime-local` value means on the account's clock; `null` when it is malformed. */
export function fromDateTimeInput(value: string, zone: string = planTimeZone()): Date | null {
  const [day = "", time = ""] = value.split("T");
  return combine(day, time.slice(0, 5), zone);
}

/** "12月24日(火)" / "Tue, Dec 24", with the year when it is not this one. */
export function formatDay(day: string, language: string, now: Date = new Date()): string {
  const date = parseYmd(day);
  if (!date) return day;
  return new Intl.DateTimeFormat(language, {
    // "This year" is the account's year.
    year: date.getFullYear() === Number(todayYmd(now).slice(0, 4)) ? undefined : "numeric",
    month: "short",
    day: "numeric",
    weekday: "short",
  }).format(date);
}

/** "18:30" / "6:30 PM": the time of an instant on the account's clock. */
export function formatTime(date: Date, language: string, zone: string = planTimeZone()): string {
  return new Intl.DateTimeFormat(language, {
    hour: "numeric",
    minute: "2-digit",
    timeZone: zone,
  }).format(date);
}

/** "2026年12月" / "December 2026". */
export function formatMonth(month: string, language: string): string {
  const date = parseYmd(`${month}-01`);
  return date
    ? new Intl.DateTimeFormat(language, { year: "numeric", month: "long" }).format(date)
    : month;
}

/** The seven weekday initials of a month page, Sunday first. */
export function weekdayLabels(language: string): string[] {
  const format = new Intl.DateTimeFormat(language, { weekday: "narrow" });
  // 2023-01-01 was a Sunday.
  return Array.from({ length: 7 }, (_, i) => format.format(new Date(2023, 0, 1 + i)));
}
