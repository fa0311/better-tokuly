import type { community } from "@tokuly/api";
import type { ClientSource } from "../query";
import { type DirectoryOptions, communityKeys as keys, type ThreadListOptions } from "./keys";

type Communities = ClientSource<typeof community.CommunityService>;
type Boards = ClientSource<typeof community.CommunityBoardService>;
type Moderation = ClientSource<typeof community.CommunityModerationService>;
type Events = ClientSource<typeof community.CommunityEventService>;
type Discovery = ClientSource<typeof community.CommunityDiscoveryService>;

const PAGE_SIZE = 20;

/** One page of a cursor-paged list, as it is cached. */
export interface Paged<T> {
  items: T[];
  nextCursor: string;
}

/** One page of a thread list: the pinned threads come with the first page only. */
export interface ThreadPage extends Paged<community.Thread> {
  pinned: community.Thread[];
}

/** One page of the caller's communities, with what the first page says about them. */
export interface MyCommunitiesPage {
  communities: community.Community[];
  requested: community.Community[];
  maxOwned: number;
  ownedCount: number;
  nextCursor: string;
}

/** One page of the communities of the caller's school, and the school's name. */
export interface SchoolPage extends Paged<community.Community> {
  schoolName: string;
}

const nextCursorOf = (page: { nextCursor: string }) => page.nextCursor || undefined;
const pageOf = (cursor: string, limit = PAGE_SIZE) => ({ limit, cursor });
const cursorOf = (response: { page?: { nextCursor: string } | undefined }) =>
  response.page?.nextCursor ?? "";

/** Infinite query options of a cursor-paged list. */
function paged<T>(queryKey: readonly unknown[], load: (cursor: string) => Promise<Paged<T>>) {
  return {
    queryKey,
    queryFn: ({ pageParam }: { pageParam: string }) => load(pageParam),
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
  };
}

/**
 * Query options of the community domain. Each app adds `enabled` and its
 * hooks around them; infinite lists are pages of `{ items, nextCursor }`
 * (threads add `pinned`), the shape the cache edits of `./cache` expect.
 */
export const communityQueries = {
  /** One community as the caller sees it; `null` when it does not exist for them. */
  community: (client: Communities, idOrHandle: string) => ({
    queryKey: keys.community(idOrHandle),
    queryFn: async () =>
      (await client().getCommunity({ communityId: idOrHandle })).community ?? null,
  }),

  my: (client: Communities) => ({
    queryKey: keys.my,
    queryFn: async ({ pageParam }: { pageParam: string }): Promise<MyCommunitiesPage> => {
      const response = await client().listMyCommunities({ page: pageOf(pageParam, 50) });
      return {
        communities: response.communities,
        requested: response.requested,
        maxOwned: response.maxOwned,
        ownedCount: response.ownedCount,
        nextCursor: cursorOf(response),
      };
    },
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
  }),

  ofUser: (client: Communities, userId: string) =>
    paged(keys.ofUser(userId), async (cursor) => {
      const response = await client().listUserCommunities({ userId, page: pageOf(cursor, 12) });
      return { items: response.communities, nextCursor: cursorOf(response) };
    }),

  members: (client: Communities, communityId: string, query: string, role: community.MemberRole) =>
    paged(keys.members(communityId, query, role), async (cursor) => {
      const response = await client().listMembers({
        communityId,
        query,
        role,
        page: pageOf(cursor, 30),
      });
      return { items: response.members, nextCursor: cursorOf(response) };
    }),

  invites: (client: Communities, communityId: string) =>
    paged(keys.invites(communityId), async (cursor) => {
      const response = await client().listInvites({ communityId, page: pageOf(cursor) });
      return { items: response.invites, nextCursor: cursorOf(response) };
    }),

  joinRequests: (client: Communities, communityId: string) =>
    paged(keys.requests(communityId), async (cursor) => {
      const response = await client().listJoinRequests({ communityId, page: pageOf(cursor) });
      return { items: response.requests, nextCursor: cursorOf(response) };
    }),

  /** What an invite code opens, and who made it. */
  invitePreview: (client: Communities, code: string) => ({
    queryKey: keys.invitePreview(code),
    queryFn: () => client().previewInvite({ inviteCode: code }),
  }),

  // ───────────── Discovery ─────────────

  directory: (client: Discovery, options: DirectoryOptions) =>
    paged(keys.directory(options), async (cursor) => {
      const response = await client().listDirectory({ ...options, page: pageOf(cursor) });
      return { items: response.communities, nextCursor: cursorOf(response) };
    }),

  trending: (client: Discovery, category: community.CommunityCategory) => ({
    queryKey: keys.trending(category),
    queryFn: async () => (await client().listTrending({ category, limit: 12 })).communities,
    staleTime: 5 * 60_000,
  }),

  friends: (client: Discovery) =>
    paged(keys.friends, async (cursor) => {
      const response = await client().listFriendsCommunities({ page: pageOf(cursor, 12) });
      return { items: response.communities, nextCursor: cursorOf(response) };
    }),

  school: (client: Discovery) => ({
    queryKey: keys.school,
    queryFn: async ({ pageParam }: { pageParam: string }): Promise<SchoolPage> => {
      const response = await client().listSchoolCommunities({ page: pageOf(pageParam, 12) });
      return {
        items: response.communities,
        schoolName: response.schoolName,
        nextCursor: cursorOf(response),
      };
    },
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
  }),

  popularTags: (client: Discovery, category: community.CommunityCategory) => ({
    queryKey: keys.tags(category),
    queryFn: async () => (await client().listPopularTags({ category, limit: 24 })).tags,
    staleTime: 5 * 60_000,
  }),

  // ───────────── Boards ─────────────

  channels: (client: Boards, communityId: string) => ({
    queryKey: keys.channels(communityId),
    queryFn: async () => (await client().listChannels({ communityId })).channels,
  }),

  threads: (client: Boards, communityId: string, options: ThreadListOptions) => ({
    queryKey: keys.threads(communityId, options),
    queryFn: async ({ pageParam }: { pageParam: string }): Promise<ThreadPage> => {
      const response = await client().listThreads({
        communityId,
        ...options,
        page: pageOf(pageParam),
      });
      return { pinned: response.pinned, items: response.threads, nextCursor: cursorOf(response) };
    },
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
  }),

  announcements: (client: Boards, communityId: string) =>
    paged(keys.announcements(communityId), async (cursor) => {
      const response = await client().listAnnouncements({ communityId, page: pageOf(cursor) });
      return { items: response.threads, nextCursor: cursorOf(response) };
    }),

  followed: (client: Boards) =>
    paged(keys.followed, async (cursor) => {
      const response = await client().listFollowedThreads({ page: pageOf(cursor) });
      return { items: response.threads, nextCursor: cursorOf(response) };
    }),

  /** One thread; `null` when it is gone (deleted, hidden) or not the caller's to read. */
  thread: (client: Boards, threadId: string) => ({
    queryKey: keys.thread(threadId),
    queryFn: async () => (await client().getThread({ threadId })).thread ?? null,
  }),

  replies: (client: Boards, threadUuid: string, newestFirst: boolean) =>
    paged(keys.replies(threadUuid, newestFirst), async (cursor) => {
      const response = await client().listReplies({
        threadId: threadUuid,
        newestFirst,
        page: pageOf(cursor, 30),
      });
      return { items: response.replies, nextCursor: cursorOf(response) };
    }),

  // ───────────── Events ─────────────

  events: (client: Events, communityId: string, past: boolean) =>
    paged(keys.events(communityId, past), async (cursor) => {
      const response = await client().listEvents({ communityId, past, page: pageOf(cursor) });
      return { items: response.events, nextCursor: cursorOf(response) };
    }),

  myEvents: (client: Events) =>
    paged(keys.myEvents, async (cursor) => {
      const response = await client().listMyEvents({ page: pageOf(cursor) });
      return { items: response.events, nextCursor: cursorOf(response) };
    }),

  /** One event; `null` when it was removed or is not the caller's to read. */
  event: (client: Events, eventId: string) => ({
    queryKey: keys.event(eventId),
    queryFn: async () => (await client().getEvent({ eventId })).event ?? null,
  }),

  attendees: (
    client: Events,
    eventId: string,
    status: community.AttendanceStatus,
    checkedInOnly: boolean,
  ) =>
    paged(keys.attendees(eventId, status, checkedInOnly), async (cursor) => {
      const response = await client().listAttendees({
        eventId,
        status,
        checkedInOnly,
        page: pageOf(cursor, 30),
      });
      return { items: response.attendees, nextCursor: cursorOf(response) };
    }),

  // ───────────── Staff ─────────────

  bans: (client: Moderation, communityId: string) =>
    paged(keys.bans(communityId), async (cursor) => {
      const response = await client().listBans({ communityId, page: pageOf(cursor) });
      return { items: response.bans, nextCursor: cursorOf(response) };
    }),

  safety: (client: Moderation, communityId: string) => ({
    queryKey: keys.safety(communityId),
    queryFn: async () => (await client().getSafetySettings({ communityId })).settings,
  }),

  reviewQueue: (client: Moderation, communityId: string) =>
    paged(keys.queue(communityId), async (cursor) => {
      const response = await client().listReviewQueue({ communityId, page: pageOf(cursor) });
      return { items: response.items, nextCursor: cursorOf(response) };
    }),

  moderationLog: (client: Moderation, communityId: string, action: community.ModerationAction) =>
    paged(keys.log(communityId, action), async (cursor) => {
      const response = await client().listModerationLog({
        communityId,
        action,
        page: pageOf(cursor, 30),
      });
      return { items: response.entries, nextCursor: cursorOf(response) };
    }),
};
