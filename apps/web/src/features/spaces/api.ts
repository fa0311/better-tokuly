import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { social, spaces } from "@tokuly/api";
import { spacesQueries } from "@tokuly/core/spaces";
import { api } from "#/lib/api";

export const spaceClient = () => api().service(spaces.SpaceService);
export const hangoutClient = () => api().service(spaces.HangoutService);

/** What this server offers: whether audio works at all, and its limits. */
export function useSpacesConfig() {
  return useQuery(spacesQueries.config(spaceClient));
}

/** One room as the caller sees it; `null` when there is none for them (gone, private, banned). */
export function useSpace(spaceId: string) {
  return useQuery(spacesQueries.space(spaceClient, spaceId));
}

export function useLiveSpaces(scope: spaces.LiveScope, tag = "", language = "") {
  return useInfiniteQuery(spacesQueries.live(spaceClient, scope, tag, language));
}

export function useUpcomingSpaces(tag = "", language = "", remindersOnly = false) {
  return useInfiniteQuery(spacesQueries.upcoming(spaceClient, tag, language, remindersOnly));
}

export function useMySpaces() {
  return useInfiniteQuery(spacesQueries.mine(spaceClient));
}

export function usePopularTags(language = "") {
  return useQuery(spacesQueries.tags(spaceClient, language));
}

/** Friends who are inside a room or hangout the caller could enter right now. */
export function useFriendsInRooms(enabled = true) {
  return useQuery({ ...spacesQueries.friendsInRooms(spaceClient), enabled });
}

/**
 * The people inside a room (or, with PAST, who is gone). `data` is every
 * loaded page in one list; `hiddenCount` are listeners who are only counted.
 */
export function useParticipants(spaceId: string, filter: spaces.ParticipantFilter, enabled = true) {
  const query = useInfiniteQuery({
    ...spacesQueries.participants(spaceClient, spaceId, filter),
    enabled,
  });
  const pages = query.data?.pages ?? [];
  return {
    data: query.data ? pages.flatMap((page) => page.participants) : undefined,
    hiddenCount: pages.reduce((sum, page) => sum + page.hiddenCount, 0),
    paging: query,
  };
}

export function useBans(spaceId: string, enabled: boolean) {
  return useInfiniteQuery({ ...spacesQueries.bans(spaceClient, spaceId), enabled });
}

export function useSpaceInvites(spaceId: string, enabled: boolean) {
  return useInfiniteQuery({ ...spacesQueries.invites(spaceClient, spaceId), enabled });
}

export function useHangouts(enabled = true) {
  return useQuery({ ...spacesQueries.hangouts(hangoutClient), enabled });
}

/** One hangout; `null` when the caller is no part of it (any more). */
export function useHangout(hangoutId: string) {
  return useQuery(spacesQueries.hangout(hangoutClient, hangoutId));
}

/** Every friend of the caller, for the pickers (the cache is shared with other areas' pickers). */
export function useFriends(enabled = true) {
  return useQuery({
    queryKey: ["social", "friends", "all"],
    queryFn: async () => {
      const friends: social.Friend[] = [];
      let cursor = "";
      do {
        const page = await api()
          .service(social.FriendService)
          .listFriends({ page: { limit: 100, cursor } });
        friends.push(...page.friends);
        cursor = page.page?.nextCursor ?? "";
      } while (cursor);
      return friends;
    },
    staleTime: 60_000,
    enabled,
  });
}
