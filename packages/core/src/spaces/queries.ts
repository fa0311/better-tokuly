import { isNotFound, type spaces } from "@tokuly/api";
import { type ClientSource, journal } from "../query";
import { spacesKeys } from "./keys";

type Spaces = ClientSource<typeof spaces.SpaceService>;
type Hangouts = ClientSource<typeof spaces.HangoutService>;

const PAGE_SIZE = 30;

/** One page of a list of rooms, as it is cached. */
export interface SpacePage {
  spaces: spaces.Space[];
  nextCursor: string;
}

/** One page of the people inside, with how many listeners the page leaves unnamed. */
export interface ParticipantPage {
  participants: spaces.SpaceParticipant[];
  hiddenCount: number;
  nextCursor: string;
}

/** The caller's hangouts and the invitations waiting for an answer. */
export interface HangoutBoard {
  hangouts: spaces.Hangout[];
  invitations: spaces.Hangout[];
}

const nextCursorOf = (page: { nextCursor: string }) => page.nextCursor || undefined;

function toPage(response: {
  spaces: spaces.Space[];
  page?: { nextCursor: string } | undefined;
}): SpacePage {
  return { spaces: response.spaces, nextCursor: response.page?.nextCursor ?? "" };
}

/**
 * Query options of rooms and hangouts. Each app adds `enabled` and its hooks
 * around them. What realtime changes reads through the journal (`pushTo` in `./cache`).
 */
export const spacesQueries = {
  /** What this server offers: whether audio works at all, and its limits. */
  config: (client: Spaces) => ({
    queryKey: spacesKeys.config,
    queryFn: async () => (await client().getSpacesConfig({})).config ?? null,
    staleTime: 10 * 60_000,
  }),

  /** One room as the caller sees it; `null` when there is none for them (gone, private, banned). */
  space: (client: Spaces, spaceId: string) => ({
    queryKey: spacesKeys.space(spaceId),
    persister: journal,
    queryFn: async ({ signal }: { signal: AbortSignal }): Promise<spaces.Space | null> => {
      try {
        return (await client().getSpace({ spaceId }, { signal })).space ?? null;
      } catch (error) {
        if (isNotFound(error)) return null;
        throw error;
      }
    },
  }),

  live: (client: Spaces, scope: spaces.LiveScope, tag = "", language = "") => ({
    queryKey: spacesKeys.live(scope, tag, language),
    queryFn: async ({ pageParam, signal }: { pageParam: string; signal: AbortSignal }) =>
      toPage(
        await client().listLiveSpaces(
          { scope, tag, language, page: { limit: PAGE_SIZE, cursor: pageParam } },
          { signal },
        ),
      ),
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
    persister: journal,
  }),

  upcoming: (client: Spaces, tag = "", language = "", remindersOnly = false) => ({
    queryKey: spacesKeys.upcoming(tag, language, remindersOnly),
    queryFn: async ({ pageParam, signal }: { pageParam: string; signal: AbortSignal }) =>
      toPage(
        await client().listUpcomingSpaces(
          { tag, language, remindersOnly, page: { limit: PAGE_SIZE, cursor: pageParam } },
          { signal },
        ),
      ),
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
    persister: journal,
  }),

  mine: (client: Spaces) => ({
    queryKey: spacesKeys.mine,
    queryFn: async ({ pageParam, signal }: { pageParam: string; signal: AbortSignal }) =>
      toPage(
        await client().listMySpaces({ page: { limit: PAGE_SIZE, cursor: pageParam } }, { signal }),
      ),
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
    persister: journal,
  }),

  tags: (client: Spaces, language = "") => ({
    queryKey: spacesKeys.tags(language),
    queryFn: async () => (await client().listPopularTags({ language, limit: 20 })).tags,
    staleTime: 30_000,
  }),

  /** Friends who are inside a room or hangout the caller could enter right now. */
  friendsInRooms: (client: Spaces) => ({
    queryKey: spacesKeys.friendsInRooms,
    queryFn: async () => (await client().listFriendsInRooms({})).friends,
    staleTime: 30_000,
  }),

  /** The people inside a room (or, with PAST, who is gone), a page at a time. */
  participants: (client: Spaces, spaceId: string, filter: spaces.ParticipantFilter) => ({
    queryKey: spacesKeys.participants(spaceId, filter),
    queryFn: async ({
      pageParam,
      signal,
    }: {
      pageParam: string;
      signal: AbortSignal;
    }): Promise<ParticipantPage> => {
      const response = await client().listParticipants(
        { spaceId, filter, page: { limit: 60, cursor: pageParam } },
        { signal },
      );
      return {
        participants: response.participants,
        hiddenCount: response.hiddenCount,
        nextCursor: response.page?.nextCursor ?? "",
      };
    },
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
    persister: journal,
  }),

  bans: (client: Spaces, spaceId: string) => ({
    queryKey: spacesKeys.bans(spaceId),
    queryFn: async ({ pageParam }: { pageParam: string }) => {
      const response = await client().listBans({
        spaceId,
        page: { limit: PAGE_SIZE, cursor: pageParam },
      });
      return { bans: response.bans, nextCursor: response.page?.nextCursor ?? "" };
    },
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
  }),

  invites: (client: Spaces, spaceId: string) => ({
    queryKey: spacesKeys.invites(spaceId),
    queryFn: async ({ pageParam }: { pageParam: string }) => {
      const response = await client().listSpaceInvites({
        spaceId,
        page: { limit: PAGE_SIZE, cursor: pageParam },
      });
      return { invites: response.invites, nextCursor: response.page?.nextCursor ?? "" };
    },
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
  }),

  /** The caller's hangouts and invitations. Pushes reach it through `pushTo` (`applySpacesEvent`). */
  hangouts: (client: Hangouts) => ({
    queryKey: spacesKeys.hangouts,
    queryFn: async ({ signal }: { signal: AbortSignal }): Promise<HangoutBoard> => {
      const response = await client().listHangouts({}, { signal });
      return { hangouts: response.hangouts, invitations: response.invitations };
    },
    persister: journal,
  }),

  /**
   * One hangout; `null` when the caller is no part of it (any more). Pushes
   * reach it through `pushTo` (`applySpacesEvent`).
   */
  hangout: (client: Hangouts, hangoutId: string) => ({
    queryKey: spacesKeys.hangout(hangoutId),
    persister: journal,
    queryFn: async ({ signal }: { signal: AbortSignal }): Promise<spaces.Hangout | null> => {
      try {
        return (await client().getHangout({ hangoutId }, { signal })).hangout ?? null;
      } catch (error) {
        if (isNotFound(error)) return null;
        throw error;
      }
    },
  }),
};
