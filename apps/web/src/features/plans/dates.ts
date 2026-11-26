import * as dates from "@tokuly/core/plans";
import { accountTimeZone } from "#/lib/account-time";

/**
 * The days and times of plans (`@tokuly/core/plans`) on the account's
 * calendar and clock (its time zone, `lib/account-time`), wherever the device
 * is. Every zone argument defaults to the account's.
 */

export {
  addDays,
  addMonths,
  daysBetween,
  formatMonth,
  monthGrid,
  monthOf,
  parseYmd,
  weekdayLabels,
} from "@tokuly/core/plans";

/** The time zone plans work in and name in requests: the account's. */
export function planTimeZone(): string {
  return accountTimeZone();
}

/** `YYYY-MM-DD` of an instant on the account's calendar. */
export function ymd(date: Date, zone: string = planTimeZone()): string {
  return dates.ymd(date, zone);
}

/** Today on the account's calendar (its time zone, not the device's): what the API's dates are compared with. */
export function todayYmd(now: Date = new Date()): string {
  return dates.todayYmd(planTimeZone(), now);
}

/** `HH:MM` of an instant on the account's clock (the value of a time input). */
export function timeOf(date: Date, zone: string = planTimeZone()): string {
  return dates.timeOf(date, zone);
}

/** A day and a `HH:MM` time on the account's clock as an instant; `null` when either is malformed. */
export function combine(day: string, time: string, zone: string = planTimeZone()): Date | null {
  return dates.combine(day, time, zone);
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

/** "12月24日(火)" / "Tue, Dec 24", with the year when it is not this one (the account's year). */
export function formatDay(day: string, language: string, now: Date = new Date()): string {
  return dates.formatDay(day, language, todayYmd(now));
}

/** "18:30" / "6:30 PM": the time of an instant on the account's clock. */
export function formatTime(date: Date, language: string, zone: string = planTimeZone()): string {
  return dates.formatTime(date, language, zone);
}
