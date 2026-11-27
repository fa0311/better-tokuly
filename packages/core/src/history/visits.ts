import { type geo, toDate } from "@tokuly/api";
import type { i18n as I18n } from "i18next";
import { compactDuration, zonedDate } from "../format";
import { placeEmoji } from "../places/model";

/** The emoji a visit is drawn with: its place's, a tick for a check-in, footprints otherwise. */
export const visitEmoji = (visit: geo.Visit): string =>
  visit.place ? placeEmoji(visit.place) : visit.checkIn ? "✅" : "👣";

/** What to call a visit: the place, else the name the server gave the spot. */
export const visitName = (visit: geo.Visit): string =>
  visit.place?.name || visit.name || visit.locality;

const clock = (date: Date | undefined, language: string, zone: string) =>
  date?.toLocaleTimeString(language, { timeZone: zone, hour: "2-digit", minute: "2-digit" }) ?? "";

/**
 * "17:16–17:25・9分", "17:33〜 いまここ" or just the time of a check-in, on
 * the clock of `zone` (the account's), in the language of `i18n`.
 */
export function visitTimes(i18n: I18n, visit: geo.Visit, zone: string): string {
  const language = i18n.language;
  const arrivedAt = toDate(visit.arrivedAt);
  const departed = toDate(visit.departedAt);
  // A stay that began on an earlier day (a night at home) says which day.
  const overnight =
    arrivedAt &&
    departed &&
    zonedDate(arrivedAt.getTime(), zone) !== zonedDate(departed.getTime(), zone);
  const arrived = `${overnight ? `${arrivedAt.toLocaleDateString(language, { timeZone: zone, month: "numeric", day: "numeric" })} ` : ""}${clock(arrivedAt, language, zone)}`;
  const seconds = Number(visit.durationSeconds);
  if (departed)
    return seconds >= 60
      ? `${arrived}–${clock(departed, language, zone)}・${compactDuration(language, seconds * 1000)}`
      : arrived;
  return visit.checkIn && seconds < 60
    ? arrived
    : i18n.t("history:visit.ongoing", { time: arrived });
}
