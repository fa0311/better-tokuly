/**
 * Days and times for plans. A day without a time is a `YYYY-MM-DD` string
 * (the API's `*_date` fields) and always means that day where the person is;
 * nothing here turns such a day into an instant.
 */

/** `YYYY-MM-DD` of a date, in the device's time zone. */
export function ymd(date: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

export function todayYmd(now: Date = new Date()): string {
  return ymd(now);
}

/** Local midnight of a `YYYY-MM-DD` day; `null` when the text is not a real day. */
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
  return ymd(date);
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
  let cursor = addDays(ymd(first), -first.getDay());
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

/** `HH:MM` of a date, in the device's time zone (the value of a time input). */
export function timeOf(date: Date): string {
  return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

/** A day and a `HH:MM` time as an instant where the person is; `null` when either is malformed. */
export function combine(day: string, time: string): Date | null {
  const date = parseYmd(day);
  const match = /^(\d{2}):(\d{2})$/.exec(time);
  if (!date || !match) return null;
  date.setHours(Number(match[1]), Number(match[2]), 0, 0);
  return date;
}

/** The device's IANA time zone, e.g. `Asia/Tokyo`. */
export function localTimeZone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone;
}

/** "12月24日(火)" / "Tue, Dec 24", with the year when it is not this one. */
export function formatDay(day: string, language: string, now: Date = new Date()): string {
  const date = parseYmd(day);
  if (!date) return day;
  return new Intl.DateTimeFormat(language, {
    year: date.getFullYear() === now.getFullYear() ? undefined : "numeric",
    month: "short",
    day: "numeric",
    weekday: "short",
  }).format(date);
}

/** "18:30" / "6:30 PM". */
export function formatTime(date: Date, language: string): string {
  return new Intl.DateTimeFormat(language, { hour: "numeric", minute: "2-digit" }).format(date);
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
