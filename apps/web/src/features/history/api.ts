import { geo } from "@tokuly/api";
import { api } from "#/lib/api";
import type { Month } from "./dates";

export const historyClient = () => api().service(geo.HistoryService);

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
