import { fromDate, type geo } from "@tokuly/api";
import { type ClientSource, nextCursor } from "../query";
import { dayRange, type Month } from "./dates";
import { historyKeys as keys } from "./keys";

type History = ClientSource<typeof geo.HistoryService>;

/**
 * Query options of the history screens on every app. The history is only
 * ever the caller's own. Days are cut in `timeZone`, the account's. Each app
 * adds `enabled` and its hooks around them.
 */
export const historyQueries = {
  /** Which days of a month have something. */
  month: (client: History, month: Month, timeZone: string) => ({
    queryKey: keys.month(month, timeZone),
    queryFn: () => client().listActiveDays({ year: month.year, month: month.month, timeZone }),
  }),
  day: (client: History, date: string, timeZone: string) => ({
    queryKey: keys.day(date, timeZone),
    queryFn: () => client().getDay({ date, timeZone }),
  }),
  /**
   * The points of a day with their times, the day being cut in `timeZone`
   * (`null` for a date that is none: the app does not enable the query).
   * When the store that keeps them cannot be reached (HISTORY_UNAVAILABLE)
   * the day is shown without them, so it is not asked again.
   */
  route: (client: History, date: string, timeZone: string, maxPoints = 1500) => {
    const range = dayRange(date, timeZone);
    return {
      queryKey: keys.route(date, timeZone),
      queryFn: () =>
        client().getRoute({
          from: range ? fromDate(range.from) : undefined,
          to: range ? fromDate(range.to) : undefined,
          maxPoints,
        }),
      enabled: range !== null,
      retry: false,
    };
  },
  /** Visits newest first, between `from` and `to` (open ends when `null`), at one place or any (""). */
  visits: (client: History, from: Date | null, to: Date | null, placeId = "") => ({
    queryKey: keys.visits(from?.getTime() ?? null, to?.getTime() ?? null, placeId),
    queryFn: ({ pageParam }: { pageParam: string }) =>
      client().listVisits({
        from: from ? fromDate(from) : undefined,
        to: to ? fromDate(to) : undefined,
        placeId,
        page: { limit: 30, cursor: pageParam },
      }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  }),
  nights: (client: History) => ({
    queryKey: keys.nights,
    queryFn: ({ pageParam }: { pageParam: string }) =>
      client().listNights({ page: { limit: 30, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  }),
  /** Totals since `from` (all time when `null`). */
  stats: (client: History, from: Date | null) => ({
    queryKey: keys.stats(from?.getTime() ?? null, null),
    queryFn: () => client().getStats({ from: from ? fromDate(from) : undefined }),
  }),
};
