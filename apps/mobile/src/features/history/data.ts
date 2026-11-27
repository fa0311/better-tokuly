import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { geo } from "@tokuly/api";
import { DEFAULT_TIME_ZONE, zonedDate } from "@tokuly/core/format";
import { dayRange, historyQueries, type Month } from "@tokuly/core/history";
import { placeQueries } from "@tokuly/core/places";
import { useEffect, useState } from "react";
import { AppState } from "react-native";
import { currentAccount, isActive } from "#/features/session/account";
import { useMe } from "#/features/session/store";
import { api } from "#/lib/api";

/**
 * The history in the query cache. The keys, query options and rules are
 * `@tokuly/core/history`; this file wires them to the phone's API client.
 * Every client is `api()` when called: whoever is signed in now. The history
 * is only ever the caller's own, it lives only in the query cache (nothing
 * of it is stored on the device or written to a log), and the cache starts
 * over with every account switch.
 */
export const historyClient = () => api().service(geo.HistoryService);
export const placeClient = () => api().service(geo.PlaceService);

/** Strings of this area (`@tokuly/i18n`). */
export const NS = "history";

/**
 * One write as the account signed in now (`send` is called at once, before
 * any await). Its answer — or its refusal — belongs to that account: once
 * another one took over it is `undefined`, so nothing of it is shown under
 * the next account (README, "Accounts").
 */
export async function asAccount<T>(send: () => Promise<T>): Promise<T | undefined> {
  const account = currentAccount();
  if (!account) return undefined;
  try {
    const answer = await send();
    return isActive(account) ? answer : undefined;
  } catch (error) {
    if (isActive(account)) throw error;
    return undefined;
  }
}

/** The account's time zone: the history's days are cut on its calendar, not the phone's. */
export function useAccountTimeZone(): string {
  return useMe()?.timeZone || DEFAULT_TIME_ZONE;
}

/** Today on the account's calendar, again every minute and whenever the app comes back to the front. */
export function useAccountToday(): string {
  const zone = useAccountTimeZone();
  const [ms, setMs] = useState(Date.now);
  useEffect(() => {
    const update = () => setMs(Date.now());
    const timer = setInterval(update, 60_000);
    const appState = AppState.addEventListener("change", (state) => {
      if (state === "active") update();
    });
    return () => {
      clearInterval(timer);
      appState.remove();
    };
  }, []);
  return zonedDate(ms, zone);
}

export function useMonth(month: Month) {
  return useQuery(historyQueries.month(historyClient, month, useAccountTimeZone()));
}

export function useDay(date: string) {
  const zone = useAccountTimeZone();
  return useQuery({
    ...historyQueries.day(historyClient, date, zone),
    enabled: dayRange(date, zone) !== null,
  });
}

/** The day's points with their times; without them the day is shown with its path or visits only. */
export function useRoute(date: string) {
  const range = dayRange(date, useAccountTimeZone());
  return useQuery({
    ...historyQueries.route(historyClient, date, range ?? { from: new Date(0), to: new Date(0) }),
    enabled: range !== null,
  });
}

export function useVisits(from: Date | null) {
  return useInfiniteQuery(historyQueries.visits(historyClient, from, null));
}

export function useNights() {
  return useInfiniteQuery(historyQueries.nights(historyClient));
}

export function useStats(from: Date | null) {
  return useQuery(historyQueries.stats(historyClient, from));
}

/** The caller's places: each carries how often they have been there. */
export function useMyPlaces() {
  return useQuery(placeQueries.mine(placeClient));
}
