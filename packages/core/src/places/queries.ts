import type { geo } from "@tokuly/api";
import { type ClientSource, nextCursor } from "../query";
import { placeKeys as keys } from "./keys";

type Places = ClientSource<typeof geo.PlaceService>;

/**
 * Query options of what the place screens read on every app. Everything is
 * the caller's view: their own places, and what of others' the server lets
 * them see. Each app adds `enabled` and its hooks around them.
 */
export const placeQueries = {
  mine: (client: Places) => ({
    queryKey: keys.mine,
    queryFn: () => client().listMyPlaces({}),
  }),
  /** Frequent stays that are not a place yet. */
  suggestions: (client: Places) => ({
    queryKey: keys.suggestions,
    queryFn: () => client().listPlaceSuggestions({}),
  }),
  saved: (client: Places) => ({
    queryKey: keys.saved,
    queryFn: ({ pageParam }: { pageParam: string }) =>
      client().listSavedPlaces({ page: { limit: 30, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  }),
  /** A place card; a place the caller may not see is PLACE_NOT_FOUND, so it is not asked twice. */
  place: (client: Places, placeId: string) => ({
    queryKey: keys.place(placeId),
    queryFn: () => client().getPlace({ placeId }),
    retry: false,
  }),
};
