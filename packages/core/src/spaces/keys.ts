import type { spaces } from "@tokuly/api";

/** Query keys of the spaces domain (rooms and hangouts). Everything starts with `"spaces"`. */
export const spacesKeys = {
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
