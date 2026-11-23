import {
  type InfiniteData,
  type QueryClient,
  useInfiniteQuery,
  useQuery,
} from "@tanstack/react-query";
import { isNotFound, social, spaces } from "@tokuly/api";
import { api } from "#/lib/api";

export const spaceClient = () => api().service(spaces.SpaceService);
export const hangoutClient = () => api().service(spaces.HangoutService);

/** Query keys of this area; all start with `spaces`. */
export const keys = {
  all: ["spaces"] as const,
  config: ["spaces", "config"] as const,
  space: (spaceId: string) => ["spaces", "space", spaceId] as const,
  /** Every list of rooms (live, upcoming, mine): pages of `{ spaces, nextCursor }`. */
  lists: ["spaces", "list"] as const,
  live: (scope: spaces.LiveScope, tag: string, language: string) =>
    ["spaces", "list", "live", scope, tag, language] as const,
  upcoming: (tag: string, language: string, remindersOnly: boolean) =>
    ["spaces", "list", "upcoming", tag, language, remindersOnly] as const,
  mine: ["spaces", "list", "mine"] as const,
  tags: (language: string) => ["spaces", "tags", language] as const,
  friendsInRooms: ["spaces", "friends-in-rooms"] as const,
  everyParticipants: (spaceId: string) => ["spaces", "participants", spaceId] as const,
  participants: (spaceId: string, filter: spaces.ParticipantFilter) =>
    ["spaces", "participants", spaceId, filter] as const,
  bans: (spaceId: string) => ["spaces", "bans", spaceId] as const,
  invites: (spaceId: string) => ["spaces", "invites", spaceId] as const,
  hangouts: ["spaces", "hangouts"] as const,
  hangout: (hangoutId: string) => ["spaces", "hangout", hangoutId] as const,
};

const PAGE_SIZE = 30;

/** One page of a list of rooms, as it is cached. */
export interface SpacePage {
  spaces: spaces.Space[];
  nextCursor: string;
}

const nextCursorOf = (page: { nextCursor: string }) => page.nextCursor || undefined;

function toPage(response: {
  spaces: spaces.Space[];
  page?: { nextCursor: string } | undefined;
}): SpacePage {
  return { spaces: response.spaces, nextCursor: response.page?.nextCursor ?? "" };
}

/** What this server offers: whether audio works at all, and its limits. */
export function useSpacesConfig() {
  return useQuery({
    queryKey: keys.config,
    queryFn: async () => (await spaceClient().getSpacesConfig({})).config ?? null,
    staleTime: 10 * 60_000,
  });
}

/** One room as the caller sees it; `null` when there is none for them (gone, private, banned). */
export function useSpace(spaceId: string) {
  return useQuery({
    queryKey: keys.space(spaceId),
    queryFn: async () => {
      try {
        return (await spaceClient().getSpace({ spaceId })).space ?? null;
      } catch (error) {
        if (isNotFound(error)) return null;
        throw error;
      }
    },
  });
}

export function useLiveSpaces(scope: spaces.LiveScope, tag = "", language = "") {
  return useInfiniteQuery({
    queryKey: keys.live(scope, tag, language),
    queryFn: async ({ pageParam }) =>
      toPage(
        await spaceClient().listLiveSpaces({
          scope,
          tag,
          language,
          page: { limit: PAGE_SIZE, cursor: pageParam },
        }),
      ),
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
  });
}

export function useUpcomingSpaces(tag = "", language = "", remindersOnly = false) {
  return useInfiniteQuery({
    queryKey: keys.upcoming(tag, language, remindersOnly),
    queryFn: async ({ pageParam }) =>
      toPage(
        await spaceClient().listUpcomingSpaces({
          tag,
          language,
          remindersOnly,
          page: { limit: PAGE_SIZE, cursor: pageParam },
        }),
      ),
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
  });
}

export function useMySpaces() {
  return useInfiniteQuery({
    queryKey: keys.mine,
    queryFn: async ({ pageParam }) =>
      toPage(await spaceClient().listMySpaces({ page: { limit: PAGE_SIZE, cursor: pageParam } })),
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
  });
}

export function usePopularTags(language = "") {
  return useQuery({
    queryKey: keys.tags(language),
    queryFn: async () => (await spaceClient().listPopularTags({ language, limit: 20 })).tags,
    staleTime: 30_000,
  });
}

/** Friends who are inside a room or hangout the caller could enter right now. */
export function useFriendsInRooms(enabled = true) {
  return useQuery({
    queryKey: keys.friendsInRooms,
    queryFn: async () => (await spaceClient().listFriendsInRooms({})).friends,
    staleTime: 30_000,
    enabled,
  });
}

/** One page of the people inside, with how many listeners the page leaves unnamed. */
export interface ParticipantPage {
  participants: spaces.SpaceParticipant[];
  hiddenCount: number;
  nextCursor: string;
}

/**
 * The people inside a room (or, with PAST, who is gone). `data` is every
 * loaded page in one list; `hiddenCount` are listeners who are only counted.
 */
export function useParticipants(spaceId: string, filter: spaces.ParticipantFilter, enabled = true) {
  const query = useInfiniteQuery({
    queryKey: keys.participants(spaceId, filter),
    queryFn: async ({ pageParam }): Promise<ParticipantPage> => {
      const response = await spaceClient().listParticipants({
        spaceId,
        filter,
        page: { limit: 60, cursor: pageParam },
      });
      return {
        participants: response.participants,
        hiddenCount: response.hiddenCount,
        nextCursor: response.page?.nextCursor ?? "",
      };
    },
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
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
  return useInfiniteQuery({
    queryKey: keys.bans(spaceId),
    queryFn: async ({ pageParam }) => {
      const response = await spaceClient().listBans({
        spaceId,
        page: { limit: PAGE_SIZE, cursor: pageParam },
      });
      return { bans: response.bans, nextCursor: response.page?.nextCursor ?? "" };
    },
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
    enabled,
  });
}

export function useSpaceInvites(spaceId: string, enabled: boolean) {
  return useInfiniteQuery({
    queryKey: keys.invites(spaceId),
    queryFn: async ({ pageParam }) => {
      const response = await spaceClient().listSpaceInvites({
        spaceId,
        page: { limit: PAGE_SIZE, cursor: pageParam },
      });
      return { invites: response.invites, nextCursor: response.page?.nextCursor ?? "" };
    },
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
    enabled,
  });
}

/** The caller's hangouts and the invitations waiting for an answer. */
export interface HangoutBoard {
  hangouts: spaces.Hangout[];
  invitations: spaces.Hangout[];
}

export function useHangouts(enabled = true) {
  return useQuery({
    queryKey: keys.hangouts,
    queryFn: async (): Promise<HangoutBoard> => {
      const response = await hangoutClient().listHangouts({});
      return { hangouts: response.hangouts, invitations: response.invitations };
    },
    enabled,
  });
}

/** One hangout; `null` when the caller is no part of it (any more). */
export function useHangout(hangoutId: string) {
  return useQuery({
    queryKey: keys.hangout(hangoutId),
    queryFn: async () => {
      try {
        return (await hangoutClient().getHangout({ hangoutId })).hangout ?? null;
      } catch (error) {
        if (isNotFound(error)) return null;
        throw error;
      }
    },
  });
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

/** Rewrites a room wherever it is loaded: its own page and every list that shows it. */
export function patchSpace(
  queryClient: QueryClient,
  spaceId: string,
  patch: (space: spaces.Space) => spaces.Space,
): void {
  queryClient.setQueryData<spaces.Space | null>(keys.space(spaceId), (space) =>
    space ? patch(space) : space,
  );
  queryClient.setQueriesData<InfiniteData<SpacePage, string>>(
    { queryKey: keys.lists },
    (list) =>
      list && {
        ...list,
        pages: list.pages.map((page) => ({
          ...page,
          spaces: page.spaces.map((space) => (space.id === spaceId ? patch(space) : space)),
        })),
      },
  );
}

/** Stores a room the server just returned (with the caller's `viewer`). */
export function rememberSpace(queryClient: QueryClient, space: spaces.Space | undefined): void {
  if (!space) return;
  queryClient.setQueryData(keys.space(space.id), space);
  patchSpace(queryClient, space.id, (loaded) => ({
    ...space,
    friendsInside: loaded.friendsInside,
  }));
}

/** Rewrites a hangout wherever it is loaded. */
export function patchHangout(
  queryClient: QueryClient,
  hangoutId: string,
  patch: (hangout: spaces.Hangout) => spaces.Hangout,
): void {
  queryClient.setQueryData<spaces.Hangout | null>(keys.hangout(hangoutId), (hangout) =>
    hangout ? patch(hangout) : hangout,
  );
  const each = (list: spaces.Hangout[]) =>
    list.map((hangout) => (hangout.id === hangoutId ? patch(hangout) : hangout));
  queryClient.setQueryData<HangoutBoard>(
    keys.hangouts,
    (board) => board && { hangouts: each(board.hangouts), invitations: each(board.invitations) },
  );
}

/** Stores a hangout the server just returned (with the caller's `viewer`). */
export function rememberHangout(queryClient: QueryClient, hangout: spaces.Hangout | undefined) {
  if (!hangout) return;
  queryClient.setQueryData(keys.hangout(hangout.id), hangout);
  patchHangout(queryClient, hangout.id, () => hangout);
}

/** Takes a hangout out of everything that is loaded (closed, quit, removed, declined). */
export function forgetHangout(queryClient: QueryClient, hangoutId: string): void {
  queryClient.setQueryData(keys.hangout(hangoutId), null);
  queryClient.setQueryData<HangoutBoard>(
    keys.hangouts,
    (board) =>
      board && {
        hangouts: board.hangouts.filter((hangout) => hangout.id !== hangoutId),
        invitations: board.invitations.filter((hangout) => hangout.id !== hangoutId),
      },
  );
}
