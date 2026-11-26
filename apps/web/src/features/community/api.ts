import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { community } from "@tokuly/api";
import {
  communityQueries,
  type DirectoryOptions,
  type ThreadListOptions,
  uniqueById,
} from "@tokuly/core/community";
import { api } from "#/lib/api";

export const communityClient = () => api().service(community.CommunityService);
export const boardClient = () => api().service(community.CommunityBoardService);
export const moderationClient = () => api().service(community.CommunityModerationService);
export const eventClient = () => api().service(community.CommunityEventService);
export const discoveryClient = () => api().service(community.CommunityDiscoveryService);

/** Strings of this area. */
export const NS = "community";

type ItemOf<Q extends { data?: unknown }> =
  NonNullable<Q["data"]> extends { pages: { items: (infer T)[] }[] } ? T : never;

/** The pages of a list as one array, plus what `<LoadMore>` needs. */
function withItems<Q extends { data?: { pages: { items: unknown[] }[] } }>(query: Q) {
  const items = query.data?.pages.flatMap((page) => page.items) as ItemOf<Q>[] | undefined;
  return { ...query, items };
}

// ───────────── Communities ─────────────

/** One community as the caller sees it; `null` when it does not exist for them. */
export function useCommunity(idOrHandle: string) {
  return useQuery(communityQueries.community(communityClient, idOrHandle));
}

export function useMyCommunities(enabled = true) {
  const query = useInfiniteQuery({ ...communityQueries.my(communityClient), enabled });
  const pages = query.data?.pages;
  return {
    ...query,
    communities: pages?.flatMap((page) => page.communities),
    requested: pages?.[0]?.requested,
    maxOwned: pages?.[0]?.maxOwned ?? 0,
    ownedCount: pages?.[0]?.ownedCount ?? 0,
  };
}

export function useUserCommunities(userId: string) {
  return withItems(useInfiniteQuery(communityQueries.ofUser(communityClient, userId)));
}

export function useMembers(communityId: string, query: string, role: community.MemberRole) {
  return withItems(
    useInfiniteQuery(communityQueries.members(communityClient, communityId, query, role)),
  );
}

export function useInvites(communityId: string) {
  return withItems(useInfiniteQuery(communityQueries.invites(communityClient, communityId)));
}

export function useJoinRequests(communityId: string) {
  return withItems(useInfiniteQuery(communityQueries.joinRequests(communityClient, communityId)));
}

// ───────────── Discovery ─────────────

export function useDirectory(options: DirectoryOptions, enabled = true) {
  return withItems(
    useInfiniteQuery({ ...communityQueries.directory(discoveryClient, options), enabled }),
  );
}

export function useTrending(category: community.CommunityCategory) {
  return useQuery(communityQueries.trending(discoveryClient, category));
}

export function useFriendsCommunities() {
  return withItems(useInfiniteQuery(communityQueries.friends(discoveryClient)));
}

export function useSchoolCommunities() {
  const query = useInfiniteQuery(communityQueries.school(discoveryClient));
  return { ...withItems(query), schoolName: query.data?.pages[0]?.schoolName ?? "" };
}

export function usePopularTags(category: community.CommunityCategory) {
  return useQuery(communityQueries.popularTags(discoveryClient, category));
}

// ───────────── Boards ─────────────

export function useChannels(communityId: string, enabled = true) {
  return useQuery({ ...communityQueries.channels(boardClient, communityId), enabled });
}

export function useThreads(communityId: string, options: ThreadListOptions, enabled = true) {
  const query = useInfiniteQuery({
    ...communityQueries.threads(boardClient, communityId, options),
    enabled,
  });
  const pages = query.data?.pages;
  return {
    ...query,
    pinned: pages?.[0]?.pinned,
    // The hot order moves while it is read: a later page may repeat a thread.
    threads: pages && uniqueById(pages.flatMap((page) => page.items)),
  };
}

export function useAnnouncements(communityId: string, enabled = true) {
  return withItems(
    useInfiniteQuery({ ...communityQueries.announcements(boardClient, communityId), enabled }),
  );
}

export function useFollowedThreads() {
  return withItems(useInfiniteQuery(communityQueries.followed(boardClient)));
}

/** One thread; `null` when it is gone (deleted, hidden) or not the caller's to read. */
export function useThread(threadId: string) {
  return useQuery(communityQueries.thread(boardClient, threadId));
}

export function useReplies(threadUuid: string, newestFirst: boolean, enabled = true) {
  return withItems(
    useInfiniteQuery({
      ...communityQueries.replies(boardClient, threadUuid, newestFirst),
      enabled,
    }),
  );
}

// ───────────── Events ─────────────

export function useEvents(communityId: string, past: boolean, enabled = true) {
  return withItems(
    useInfiniteQuery({ ...communityQueries.events(eventClient, communityId, past), enabled }),
  );
}

export function useMyEvents() {
  return withItems(useInfiniteQuery(communityQueries.myEvents(eventClient)));
}

/** One event; `null` when it was removed or is not the caller's to read. */
export function useEvent(eventId: string) {
  return useQuery(communityQueries.event(eventClient, eventId));
}

export function useAttendees(
  eventId: string,
  status: community.AttendanceStatus,
  checkedInOnly = false,
  enabled = true,
) {
  return withItems(
    useInfiniteQuery({
      ...communityQueries.attendees(eventClient, eventId, status, checkedInOnly),
      enabled,
    }),
  );
}

// ───────────── Staff ─────────────

export function useBans(communityId: string) {
  return withItems(useInfiniteQuery(communityQueries.bans(moderationClient, communityId)));
}

export function useSafetySettings(communityId: string) {
  return useQuery(communityQueries.safety(moderationClient, communityId));
}

export function useReviewQueue(communityId: string) {
  return withItems(useInfiniteQuery(communityQueries.reviewQueue(moderationClient, communityId)));
}

export function useModerationLog(communityId: string, action: community.ModerationAction) {
  return withItems(
    useInfiniteQuery(communityQueries.moderationLog(moderationClient, communityId, action)),
  );
}
