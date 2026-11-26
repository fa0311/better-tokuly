import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { spaces } from "@tokuly/api";
import { spacesQueries } from "@tokuly/core/spaces";
import { api } from "#/lib/api";

/**
 * Rooms and hangouts in the query cache. The keys, query options and cache
 * edits are `@tokuly/core/spaces`; this file wires them to the phone's API
 * client. Every client is `api()` when called: whoever is signed in now.
 */
export const spaceClient = () => api().service(spaces.SpaceService);
export const hangoutClient = () => api().service(spaces.HangoutService);

/** Whether this server has audio at all, and its limits. */
export function useSpacesConfig() {
  return useQuery(spacesQueries.config(spaceClient));
}

/** One room as the viewer sees it; `null` when there is none for them. */
export function useSpace(spaceId: string) {
  return useQuery({ ...spacesQueries.space(spaceClient, spaceId), enabled: spaceId !== "" });
}

export function useLiveSpaces(scope: spaces.LiveScope) {
  return useInfiniteQuery(spacesQueries.live(spaceClient, scope));
}

export function useUpcomingSpaces() {
  return useInfiniteQuery(spacesQueries.upcoming(spaceClient));
}

/** Friends who are inside a room or hangout the viewer could enter right now. */
export function useFriendsInRooms() {
  return useQuery(spacesQueries.friendsInRooms(spaceClient));
}

/**
 * The people inside a room in one list, and how many listeners it leaves
 * unnamed (they keep their online status to themselves).
 */
export function useParticipants(spaceId: string, filter: spaces.ParticipantFilter, enabled = true) {
  const query = useInfiniteQuery({
    ...spacesQueries.participants(spaceClient, spaceId, filter),
    enabled,
  });
  const pages = query.data?.pages ?? [];
  return {
    people: pages.flatMap((page) => page.participants),
    hiddenCount: pages.reduce((sum, page) => sum + page.hiddenCount, 0),
    paging: query,
  };
}

/** The viewer's hangouts and the invitations waiting for an answer. */
export function useHangouts() {
  return useQuery(spacesQueries.hangouts(hangoutClient));
}

/** One hangout; `null` when the viewer is no part of it (any more). */
export function useHangout(hangoutId: string) {
  return useQuery({
    ...spacesQueries.hangout(hangoutClient, hangoutId),
    enabled: hangoutId !== "",
  });
}
