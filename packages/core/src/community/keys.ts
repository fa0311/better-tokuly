import type { community } from "@tokuly/api";

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
 * Query keys of the community domain; all start with `community`. Lists of
 * one community carry its UUID (what realtime events name it by) in third
 * place, so one invalidation of `communityKeys.of(id)` refreshes everything
 * loaded about it.
 */
export const communityKeys = {
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
