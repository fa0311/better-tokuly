import {
  type InfiniteData,
  type QueryClient,
  useInfiniteQuery,
  useQuery,
} from "@tanstack/react-query";
import { community } from "@tokuly/api";
import { api } from "#/lib/api";
import { uniqueById } from "./model";

export const communityClient = () => api().service(community.CommunityService);
export const boardClient = () => api().service(community.CommunityBoardService);
export const moderationClient = () => api().service(community.CommunityModerationService);
export const eventClient = () => api().service(community.CommunityEventService);
export const discoveryClient = () => api().service(community.CommunityDiscoveryService);

/** Strings of this area. */
export const NS = "community";

export interface ThreadListOptions {
  channelId: string;
  sort: community.ThreadSort;
  window: community.TopWindow;
  tag: string;
}

export interface DirectoryOptions {
  category: community.CommunityCategory;
  tag: string;
  query: string;
  sort: community.DirectorySort;
}

/**
 * Query keys; all start with `community`. Lists of one community carry its
 * UUID (what realtime events name it by) in third place, so one
 * invalidation of `keys.of(id)` refreshes everything loaded about it.
 */
export const keys = {
  all: ["community"] as const,
  my: ["community", "my"] as const,
  myEvents: ["community", "my-events"] as const,
  followed: ["community", "followed"] as const,
  /** One community by whatever the address names (handle or UUID). */
  communities: ["community", "c"] as const,
  community: (idOrHandle: string) => ["community", "c", idOrHandle.toLowerCase()] as const,
  invitePreview: (code: string) => ["community", "invite", code] as const,
  ofUser: (userId: string) => ["community", "user", userId] as const,
  directory: (options: DirectoryOptions) => ["community", "directory", options] as const,
  trending: (category: community.CommunityCategory) => ["community", "trending", category] as const,
  friends: ["community", "friends"] as const,
  school: ["community", "school"] as const,
  tags: (category: community.CommunityCategory) => ["community", "tags", category] as const,

  of: (communityId: string) => ["community", "in", communityId] as const,
  channels: (communityId: string) => ["community", "in", communityId, "channels"] as const,
  threadLists: (communityId: string) => ["community", "in", communityId, "threads"] as const,
  threads: (communityId: string, options: ThreadListOptions) =>
    ["community", "in", communityId, "threads", options] as const,
  announcements: (communityId: string) =>
    ["community", "in", communityId, "announcements"] as const,
  eventLists: (communityId: string) => ["community", "in", communityId, "events"] as const,
  events: (communityId: string, past: boolean) =>
    ["community", "in", communityId, "events", past] as const,
  memberLists: (communityId: string) => ["community", "in", communityId, "members"] as const,
  members: (communityId: string, query: string, role: community.MemberRole) =>
    ["community", "in", communityId, "members", query, role] as const,
  invites: (communityId: string) => ["community", "in", communityId, "invites"] as const,
  requests: (communityId: string) => ["community", "in", communityId, "requests"] as const,
  bans: (communityId: string) => ["community", "in", communityId, "bans"] as const,
  safety: (communityId: string) => ["community", "in", communityId, "safety"] as const,
  queue: (communityId: string) => ["community", "in", communityId, "queue"] as const,
  log: (communityId: string, action: community.ModerationAction) =>
    ["community", "in", communityId, "log", action] as const,

  /** One thread by what the address names (UUID or short ID). */
  threadsById: ["community", "thread"] as const,
  thread: (threadId: string) => ["community", "thread", threadId] as const,
  replyLists: ["community", "replies"] as const,
  /** Replies by the thread's UUID. */
  replies: (threadUuid: string, newestFirst: boolean) =>
    ["community", "replies", threadUuid, newestFirst] as const,
  repliesOf: (threadUuid: string) => ["community", "replies", threadUuid] as const,
  event: (eventId: string) => ["community", "event", eventId] as const,
  eventsById: ["community", "event"] as const,
  attendeeLists: (eventId: string) => ["community", "attendees", eventId] as const,
  attendees: (eventId: string, status: community.AttendanceStatus, checkedInOnly: boolean) =>
    ["community", "attendees", eventId, status, checkedInOnly] as const,
};

const PAGE_SIZE = 20;

interface Paged<T> {
  items: T[];
  nextCursor: string;
}
const nextCursorOf = (page: { nextCursor: string }) => page.nextCursor || undefined;

/** A cursor-paged list as one array plus what `<LoadMore>` needs. */
function usePaged<T>(
  queryKey: readonly unknown[],
  load: (cursor: string) => Promise<Paged<T>>,
  enabled = true,
) {
  const query = useInfiniteQuery({
    queryKey,
    queryFn: ({ pageParam }) => load(pageParam),
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
    enabled,
  });
  return { ...query, items: query.data?.pages.flatMap((page) => page.items) };
}

const pageOf = (cursor: string, limit = PAGE_SIZE) => ({ limit, cursor });

// ───────────── Communities ─────────────

/** One community as the caller sees it; `null` when it does not exist for them. */
export function useCommunity(idOrHandle: string) {
  return useQuery({
    queryKey: keys.community(idOrHandle),
    queryFn: async () =>
      (await communityClient().getCommunity({ communityId: idOrHandle })).community ?? null,
  });
}

export interface MyCommunities {
  communities: community.Community[];
  requested: community.Community[];
  maxOwned: number;
  ownedCount: number;
  nextCursor: string;
}

export function useMyCommunities(enabled = true) {
  const query = useInfiniteQuery({
    queryKey: keys.my,
    queryFn: async ({ pageParam }): Promise<MyCommunities> => {
      const response = await communityClient().listMyCommunities({ page: pageOf(pageParam, 50) });
      return {
        communities: response.communities,
        requested: response.requested,
        maxOwned: response.maxOwned,
        ownedCount: response.ownedCount,
        nextCursor: response.page?.nextCursor ?? "",
      };
    },
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
    enabled,
  });
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
  return usePaged(keys.ofUser(userId), async (cursor) => {
    const response = await communityClient().listUserCommunities({
      userId,
      page: pageOf(cursor, 12),
    });
    return { items: response.communities, nextCursor: response.page?.nextCursor ?? "" };
  });
}

export function useMembers(communityId: string, query: string, role: community.MemberRole) {
  return usePaged(keys.members(communityId, query, role), async (cursor) => {
    const response = await communityClient().listMembers({
      communityId,
      query,
      role,
      page: pageOf(cursor, 30),
    });
    return { items: response.members, nextCursor: response.page?.nextCursor ?? "" };
  });
}

export function useInvites(communityId: string) {
  return usePaged(keys.invites(communityId), async (cursor) => {
    const response = await communityClient().listInvites({ communityId, page: pageOf(cursor) });
    return { items: response.invites, nextCursor: response.page?.nextCursor ?? "" };
  });
}

export function useJoinRequests(communityId: string) {
  return usePaged(keys.requests(communityId), async (cursor) => {
    const response = await communityClient().listJoinRequests({
      communityId,
      page: pageOf(cursor),
    });
    return { items: response.requests, nextCursor: response.page?.nextCursor ?? "" };
  });
}

// ───────────── Discovery ─────────────

export function useDirectory(options: DirectoryOptions, enabled = true) {
  return usePaged(
    keys.directory(options),
    async (cursor) => {
      const response = await discoveryClient().listDirectory({ ...options, page: pageOf(cursor) });
      return { items: response.communities, nextCursor: response.page?.nextCursor ?? "" };
    },
    enabled,
  );
}

export function useTrending(category: community.CommunityCategory) {
  return useQuery({
    queryKey: keys.trending(category),
    queryFn: async () =>
      (await discoveryClient().listTrending({ category, limit: 12 })).communities,
    staleTime: 5 * 60_000,
  });
}

export function useFriendsCommunities() {
  return usePaged(keys.friends, async (cursor) => {
    const response = await discoveryClient().listFriendsCommunities({ page: pageOf(cursor, 12) });
    return { items: response.communities, nextCursor: response.page?.nextCursor ?? "" };
  });
}

export function useSchoolCommunities() {
  const query = useInfiniteQuery({
    queryKey: keys.school,
    queryFn: async ({ pageParam }) => {
      const response = await discoveryClient().listSchoolCommunities({
        page: pageOf(pageParam, 12),
      });
      return {
        items: response.communities,
        schoolName: response.schoolName,
        nextCursor: response.page?.nextCursor ?? "",
      };
    },
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
  });
  return {
    ...query,
    items: query.data?.pages.flatMap((page) => page.items),
    schoolName: query.data?.pages[0]?.schoolName ?? "",
  };
}

export function usePopularTags(category: community.CommunityCategory) {
  return useQuery({
    queryKey: keys.tags(category),
    queryFn: async () => (await discoveryClient().listPopularTags({ category, limit: 24 })).tags,
    staleTime: 5 * 60_000,
  });
}

// ───────────── Boards ─────────────

export function useChannels(communityId: string, enabled = true) {
  return useQuery({
    queryKey: keys.channels(communityId),
    queryFn: async () => (await boardClient().listChannels({ communityId })).channels,
    enabled,
  });
}

interface ThreadPage {
  pinned: community.Thread[];
  items: community.Thread[];
  nextCursor: string;
}

export function useThreads(communityId: string, options: ThreadListOptions, enabled = true) {
  const query = useInfiniteQuery({
    queryKey: keys.threads(communityId, options),
    queryFn: async ({ pageParam }): Promise<ThreadPage> => {
      const response = await boardClient().listThreads({
        communityId,
        ...options,
        page: pageOf(pageParam),
      });
      return {
        pinned: response.pinned,
        items: response.threads,
        nextCursor: response.page?.nextCursor ?? "",
      };
    },
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
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
  return usePaged(
    keys.announcements(communityId),
    async (cursor) => {
      const response = await boardClient().listAnnouncements({ communityId, page: pageOf(cursor) });
      return { items: response.threads, nextCursor: response.page?.nextCursor ?? "" };
    },
    enabled,
  );
}

export function useFollowedThreads() {
  return usePaged(keys.followed, async (cursor) => {
    const response = await boardClient().listFollowedThreads({ page: pageOf(cursor) });
    return { items: response.threads, nextCursor: response.page?.nextCursor ?? "" };
  });
}

/** One thread; `null` when it is gone (deleted, hidden) or not the caller's to read. */
export function useThread(threadId: string) {
  return useQuery({
    queryKey: keys.thread(threadId),
    queryFn: async () => (await boardClient().getThread({ threadId })).thread ?? null,
  });
}

export function useReplies(threadUuid: string, newestFirst: boolean, enabled = true) {
  return usePaged(
    keys.replies(threadUuid, newestFirst),
    async (cursor) => {
      const response = await boardClient().listReplies({
        threadId: threadUuid,
        newestFirst,
        page: pageOf(cursor, 30),
      });
      return { items: response.replies, nextCursor: response.page?.nextCursor ?? "" };
    },
    enabled,
  );
}

type ReplyPages = InfiniteData<Paged<community.Reply>>;

/** Changes the loaded replies of a thread, in whichever order they are shown. */
export function patchReplies(
  queryClient: QueryClient,
  threadUuid: string,
  patch: (replies: community.Reply[], last: boolean, newestFirst: boolean) => community.Reply[],
): void {
  for (const [key, data] of queryClient.getQueriesData<ReplyPages>({
    queryKey: keys.repliesOf(threadUuid),
  })) {
    if (!data) continue;
    const newestFirst = key[3] === true;
    queryClient.setQueryData<ReplyPages>(key, {
      ...data,
      pages: data.pages.map((page, index) => ({
        ...page,
        items: patch(page.items, index === data.pages.length - 1, newestFirst),
      })),
    });
  }
}

/**
 * Puts a reply the caller may read into the loaded lists: replaced where it
 * is already shown, otherwise added where new replies appear (the top of a
 * newest-first list; the end of an oldest-first one once it is fully loaded).
 */
export function upsertReply(queryClient: QueryClient, reply: community.Reply): void {
  for (const [key, data] of queryClient.getQueriesData<ReplyPages>({
    queryKey: keys.repliesOf(reply.threadId),
  })) {
    if (!data) continue;
    const newestFirst = key[3] === true;
    const known = data.pages.some((page) => page.items.some((each) => each.id === reply.id));
    const lastIndex = data.pages.length - 1;
    const complete = !data.pages[lastIndex]?.nextCursor;
    queryClient.setQueryData<ReplyPages>(key, {
      ...data,
      pages: data.pages.map((page, index) => {
        if (known)
          return {
            ...page,
            items: page.items.map((each) => (each.id === reply.id ? reply : each)),
          };
        if (newestFirst && index === 0) return { ...page, items: [reply, ...page.items] };
        if (!newestFirst && index === lastIndex && complete)
          return { ...page, items: [...page.items, reply] };
        return page;
      }),
    });
  }
}

/** Applies a change to a thread wherever it is loaded (its page and every list). */
export function patchThread(
  queryClient: QueryClient,
  threadUuid: string,
  patch: (thread: community.Thread) => community.Thread,
): void {
  for (const [key, data] of queryClient.getQueriesData<community.Thread | null>({
    queryKey: keys.threadsById,
  })) {
    if (data?.id === threadUuid) queryClient.setQueryData(key, patch(data));
  }
  const inPages = <T extends { items: community.Thread[]; pinned?: community.Thread[] }>(
    data: InfiniteData<T> | undefined,
  ) =>
    data && {
      ...data,
      pages: data.pages.map((page) => ({
        ...page,
        items: page.items.map((each) => (each.id === threadUuid ? patch(each) : each)),
        ...(page.pinned
          ? { pinned: page.pinned.map((each) => (each.id === threadUuid ? patch(each) : each)) }
          : {}),
      })),
    };
  queryClient.setQueriesData<InfiniteData<ThreadPage>>(
    {
      queryKey: keys.all,
      predicate: (query) =>
        query.queryKey[3] === "threads" ||
        query.queryKey[3] === "announcements" ||
        query.queryKey[1] === "followed",
    },
    inPages,
  );
}

// ───────────── Events ─────────────

export function useEvents(communityId: string, past: boolean, enabled = true) {
  return usePaged(
    keys.events(communityId, past),
    async (cursor) => {
      const response = await eventClient().listEvents({ communityId, past, page: pageOf(cursor) });
      return { items: response.events, nextCursor: response.page?.nextCursor ?? "" };
    },
    enabled,
  );
}

export function useMyEvents() {
  return usePaged(keys.myEvents, async (cursor) => {
    const response = await eventClient().listMyEvents({ page: pageOf(cursor) });
    return { items: response.events, nextCursor: response.page?.nextCursor ?? "" };
  });
}

/** One event; `null` when it was removed or is not the caller's to read. */
export function useEvent(eventId: string) {
  return useQuery({
    queryKey: keys.event(eventId),
    queryFn: async () => (await eventClient().getEvent({ eventId })).event ?? null,
  });
}

export function useAttendees(
  eventId: string,
  status: community.AttendanceStatus,
  checkedInOnly = false,
  enabled = true,
) {
  return usePaged(
    keys.attendees(eventId, status, checkedInOnly),
    async (cursor) => {
      const response = await eventClient().listAttendees({
        eventId,
        status,
        checkedInOnly,
        page: pageOf(cursor, 30),
      });
      return { items: response.attendees, nextCursor: response.page?.nextCursor ?? "" };
    },
    enabled,
  );
}

// ───────────── Staff ─────────────

export function useBans(communityId: string) {
  return usePaged(keys.bans(communityId), async (cursor) => {
    const response = await moderationClient().listBans({ communityId, page: pageOf(cursor) });
    return { items: response.bans, nextCursor: response.page?.nextCursor ?? "" };
  });
}

export function useSafetySettings(communityId: string) {
  return useQuery({
    queryKey: keys.safety(communityId),
    queryFn: async () => (await moderationClient().getSafetySettings({ communityId })).settings,
  });
}

export function useReviewQueue(communityId: string) {
  return usePaged(keys.queue(communityId), async (cursor) => {
    const response = await moderationClient().listReviewQueue({
      communityId,
      page: pageOf(cursor),
    });
    return { items: response.items, nextCursor: response.page?.nextCursor ?? "" };
  });
}

export function useModerationLog(communityId: string, action: community.ModerationAction) {
  return usePaged(keys.log(communityId, action), async (cursor) => {
    const response = await moderationClient().listModerationLog({
      communityId,
      action,
      page: pageOf(cursor, 30),
    });
    return { items: response.entries, nextCursor: response.page?.nextCursor ?? "" };
  });
}

/** After the caller's own standing somewhere changed: everything that shows it. */
export function refreshMembership(queryClient: QueryClient, communityId?: string): void {
  void queryClient.invalidateQueries({ queryKey: keys.communities });
  void queryClient.invalidateQueries({ queryKey: keys.my });
  void queryClient.invalidateQueries({ queryKey: keys.myEvents });
  void queryClient.invalidateQueries({ queryKey: keys.followed });
  if (communityId) void queryClient.invalidateQueries({ queryKey: keys.of(communityId) });
}

/** Writes a community the server just returned over every copy loaded under an address. */
export function storeCommunity(queryClient: QueryClient, next: community.Community): void {
  for (const [key, data] of queryClient.getQueriesData<community.Community | null>({
    queryKey: keys.communities,
  })) {
    if (data?.id === next.id) queryClient.setQueryData(key, next);
  }
  queryClient.setQueryData(keys.community(next.handle), next);
}
