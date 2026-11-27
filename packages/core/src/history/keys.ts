import type { Month } from "./dates";

/** Query keys: everything of the history starts with `["geo", "history"]`. */
export const historyKeys = {
  all: ["geo", "history"] as const,
  month: (month: Month, timeZone: string) =>
    ["geo", "history", "month", month.year, month.month, timeZone] as const,
  day: (date: string, timeZone: string) => ["geo", "history", "day", date, timeZone] as const,
  route: (date: string) => ["geo", "history", "route", date] as const,
  visits: (from: number | null, to: number | null, placeId: string) =>
    ["geo", "history", "visits", from, to, placeId] as const,
  nights: ["geo", "history", "nights"] as const,
  stats: (from: number | null, to: number | null) => ["geo", "history", "stats", from, to] as const,
};

/**
 * What a deletion of history touches: the history itself, the footprints
 * uncovered in it and the places (which count visits). After a deletion
 * each app resets these (rather than marking them stale), so nothing that
 * was deleted is shown again from memory, not even for the moment a re-read
 * takes.
 */
export const DELETED_HISTORY_KEYS = [
  historyKeys.all,
  ["geo", "footprints"],
  ["geo", "places"],
] as const;
