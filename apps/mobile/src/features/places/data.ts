import { useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { geo } from "@tokuly/api";
import { historyQueries } from "@tokuly/core/history";
import { placeKeys, placeQueries, withDismissed } from "@tokuly/core/places";
import { currentAccount, isActive } from "#/features/session/account";
import { api } from "#/lib/api";

/**
 * Places in the query cache. The keys, query options and rules are
 * `@tokuly/core/places`; this file wires them to the phone's API client.
 * Every client is `api()` when called: whoever is signed in now. Everything
 * is the caller's view as the server returns it; it lives only in the query
 * cache (nothing stored on the device, nothing logged), which starts over
 * with every account switch.
 */
export const placeClient = () => api().service(geo.PlaceService);
export const historyClient = () => api().service(geo.HistoryService);

/** Strings of this area (`@tokuly/i18n`). */
export const NS = "places";

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

export function useMyPlaces() {
  return useQuery(placeQueries.mine(placeClient));
}

export function useSuggestions() {
  return useQuery(placeQueries.suggestions(placeClient));
}

export function useSavedPlaces() {
  return useInfiniteQuery(placeQueries.saved(placeClient));
}

export function usePlace(placeId: string) {
  return useQuery(placeQueries.place(placeClient, placeId));
}

/** The caller's own last visits to a place. */
export function usePlaceVisits(placeId: string, enabled: boolean) {
  return useInfiniteQuery({
    ...historyQueries.visits(historyClient, null, null, placeId),
    enabled,
  });
}

/** Places and spots by name (no position is sent: the phone's location stays out of the search). */
export function usePlaceSearch(query: string) {
  return useQuery({
    queryKey: placeKeys.search(query, null),
    queryFn: () => placeClient().searchPlaces({ query, limit: 20 }),
    enabled: query !== "",
    staleTime: 60_000,
  });
}

/**
 * Suggestions put off ("あとで"), as fingerprints. Kept in the query cache
 * only: they go with the account at the next switch and never reach the
 * device's storage, so nothing on the phone says where an account has been.
 */
const DISMISSED_KEY = ["places-dismissed"] as const;

export function useDismissed(userId: string) {
  const queryClient = useQueryClient();
  const { data } = useQuery({
    queryKey: DISMISSED_KEY,
    queryFn: () => [] as string[],
    initialData: [] as string[],
    staleTime: Number.POSITIVE_INFINITY,
  });
  const dismiss = (suggestion: geo.PlaceSuggestion, current: geo.PlaceSuggestion[]) =>
    queryClient.setQueryData<string[]>(DISMISSED_KEY, (marks = []) =>
      withDismissed(userId, marks, suggestion, current),
    );
  return { dismissed: data, dismiss };
}
