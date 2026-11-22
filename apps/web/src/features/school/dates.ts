import { school } from "@tokuly/api";
import { zonedMoment, zonedTime } from "#/lib/account-time";

/**
 * Calendar dates of the school API are `YYYY-MM-DD` strings and times of day
 * are minutes after midnight. These helpers do the arithmetic on them without
 * ever going through UTC, so a date never slips by a day.
 */

const pad = (value: number) => String(value).padStart(2, "0");

/** The calendar date of a moment on this device. "Today" for the API is the account's: see `#/lib/account-time`. */
export function ymd(date: Date): string {
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** Local midnight of a `YYYY-MM-DD` date. */
export function parseYmd(value: string): Date {
  const [year = 1970, month = 1, day = 1] = value.split("-").map(Number);
  return new Date(year, month - 1, day);
}

export function isYmd(value: string): boolean {
  return /^\d{4}-\d{2}-\d{2}$/.test(value) && ymd(parseYmd(value)) === value;
}

export function addDays(value: string, days: number): string {
  const date = parseYmd(value);
  date.setDate(date.getDate() + days);
  return ymd(date);
}

/** Whole days from `from` to `to` (negative when `to` is earlier). */
export function daysBetween(from: string, to: string): number {
  return Math.round((parseYmd(to).getTime() - parseYmd(from).getTime()) / 86_400_000);
}

/** The weekday of a date as the API numbers them (Monday = 1 … Sunday = 7). */
export function weekdayOf(value: string): school.Weekday {
  return ((parseYmd(value).getDay() + 6) % 7) + 1;
}

/** The Monday of the week a date is in (weeks start on Monday). */
export function mondayOf(value: string): string {
  return addDays(value, 1 - weekdayOf(value));
}

/** Minutes after midnight → "8:50". */
export function clock(minute: number): string {
  return `${Math.floor(minute / 60)}:${pad(minute % 60)}`;
}

/** "08:50" (a time input's value) → minutes after midnight; `undefined` when it is not a time. */
export function parseClock(value: string): number | undefined {
  const match = /^(\d{1,2}):(\d{2})$/.exec(value.trim());
  if (!match) return undefined;
  const minutes = Number(match[1]) * 60 + Number(match[2]);
  return minutes < 24 * 60 && Number(match[2]) < 60 ? minutes : undefined;
}

/** Minutes after midnight → "08:50", the value a time input wants. */
export function clockInput(minute: number): string {
  return `${pad(Math.floor(minute / 60))}:${pad(minute % 60)}`;
}

/** A running clock: "04:07", "1:02:03". */
export function timerText(totalSeconds: number): string {
  const seconds = Math.max(0, Math.floor(totalSeconds));
  const hours = Math.floor(seconds / 3600);
  const rest = `${pad(Math.floor((seconds % 3600) / 60))}:${pad(seconds % 60)}`;
  return hours > 0 ? `${hours}:${rest}` : rest;
}

/** Study time for people: "45分", "1時間20分", "2時間" (ja); "45m", "1h 20m", "2h" (en). Under a minute is "1分未満" / "<1m". */
export function studyTime(totalSeconds: number, language: string): string {
  const ja = language.startsWith("ja");
  if (totalSeconds <= 0) return ja ? "0分" : "0m";
  if (totalSeconds < 60) return ja ? "1分未満" : "<1m";
  const minutes = Math.floor(totalSeconds / 60);
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  if (hours === 0) return ja ? `${rest}分` : `${rest}m`;
  if (rest === 0) return ja ? `${hours}時間` : `${hours}h`;
  return ja ? `${hours}時間${rest}分` : `${hours}h ${rest}m`;
}

/** "5/12(火)" / "Tue, May 12". */
export function dayLabel(value: string, language: string): string {
  const date = parseYmd(value);
  if (language.startsWith("ja")) return `${date.getMonth() + 1}/${date.getDate()}(${"日月火水木金土"[date.getDay()]})`;
  return date.toLocaleDateString("en", { weekday: "short", month: "short", day: "numeric" });
}

/** A moment as "5/12(火) 17:00" / "Tue, May 12, 5:00 PM", on the clock of `zone` (the account's). */
export function momentLabel(date: Date, language: string, zone: string): string {
  const shown = zonedTime(date.getTime(), zone);
  const time = language.startsWith("ja") ? clock(shown.minute) : new Date(2000, 0, 1, 0, shown.minute).toLocaleTimeString("en", { hour: "numeric", minute: "2-digit" });
  return `${dayLabel(shown.date, language)} ${time}`;
}

/** A moment as the value of a `datetime-local` input, read on the clock of `zone` (the account's), and back. */
export function toLocalInput(date: Date, zone: string): string {
  const shown = zonedTime(date.getTime(), zone);
  return `${shown.date}T${clockInput(shown.minute)}`;
}

export function fromLocalInput(value: string, zone: string): Date | undefined {
  const match = /^(\d{4}-\d{2}-\d{2})T(\d{2}):(\d{2})/.exec(value);
  if (!match?.[1] || !isYmd(match[1])) return undefined;
  return new Date(zonedMoment(match[1], Number(match[2]) * 60 + Number(match[3]), zone));
}

/** Monday first, the order the grid draws its columns in. */
export const WEEKDAYS: school.Weekday[] = [
  school.Weekday.MONDAY,
  school.Weekday.TUESDAY,
  school.Weekday.WEDNESDAY,
  school.Weekday.THURSDAY,
  school.Weekday.FRIDAY,
  school.Weekday.SATURDAY,
  school.Weekday.SUNDAY,
];

/** The weekdays a layout has classes on (Monday to Friday when it names none), in order. */
export function layoutWeekdays(layout: school.PeriodLayout | undefined): school.Weekday[] {
  const days = layout?.weekdays.length ? layout.weekdays : WEEKDAYS.slice(0, 5);
  return WEEKDAYS.filter((day) => days.includes(day));
}
