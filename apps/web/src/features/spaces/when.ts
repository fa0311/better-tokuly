import { shiftDate, zonedDate, zonedTime } from "#/lib/account-time";
import i18n from "#/lib/i18n";

/** Minutes after midnight → "18:05", the value of a time input. */
export function clockOf(minute: number): string {
  return `${String(Math.floor(minute / 60)).padStart(2, "0")}:${String(minute % 60).padStart(2, "0")}`;
}

/** "18:05" → minutes after midnight; `null` for anything else. */
export function minuteOf(clock: string): number | null {
  const match = /^(\d{1,2}):(\d{2})$/.exec(clock);
  if (!match) return null;
  const minute = Number(match[1]) * 60 + Number(match[2]);
  return minute < 24 * 60 ? minute : null;
}

/**
 * When a room is planned for, on the account's clock: "今日 18:00",
 * "あした 9:30", "12月3日(木) 20:00". `today` is the account's date.
 */
export function formatWhen(ms: number, zone: string, today: string, language: string): string {
  const { date, minute } = zonedTime(ms, zone);
  const time = `${Math.floor(minute / 60)}:${String(minute % 60).padStart(2, "0")}`;
  if (date === today) return i18n.t("spaces:when.today", { time });
  if (date === shiftDate(today, 1)) return i18n.t("spaces:when.tomorrow", { time });
  const day = new Intl.DateTimeFormat(language, {
    month: "short",
    day: "numeric",
    weekday: "short",
    timeZone: zone,
  }).format(ms);
  return `${day} ${time}`;
}

/** The date and time inputs' values for a moment, on the account's clock. */
export function inputsOf(ms: number, zone: string): { date: string; clock: string } {
  const { date, minute } = zonedTime(ms, zone);
  return { date, clock: clockOf(minute) };
}

/** The next full hour at least half an hour away: what the schedule form starts from. */
export function suggestedStart(now: number, zone: string): { date: string; clock: string } {
  const hour = 3_600_000;
  const start = Math.ceil((now + hour / 2) / hour) * hour;
  return { date: zonedDate(start, zone), clock: clockOf(zonedTime(start, zone).minute) };
}
