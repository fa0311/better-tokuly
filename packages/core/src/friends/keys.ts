import type { social } from "@tokuly/api";

/**
 * Query keys of friends. Everything social starts with `["social"]`, so one
 * invalidation after an answer to a request re-reads all of it.
 */
export const friendKeys = {
  all: ["social"] as const,
  /** The viewer's friends filtered by name or handle (`""`: all of them); the map's list too. */
  list: (query: string) => ["social", "friends", "list", query] as const,
  requests: ["social", "requests"] as const,
  incoming: ["social", "requests", "incoming"] as const,
  outgoing: ["social", "requests", "outgoing"] as const,
  /** Prefix for suggestions; keep each server-side result limit in its own cache entry. */
  suggestions: ["social", "suggestions"] as const,
  suggestionList: (limit: number) => ["social", "suggestions", "list", limit] as const,
  invite: ["social", "invite"] as const,
};

/**
 * Query keys of follows and circles. Everything starts with
 * `["social", "follow"]` or `["social", "circles"]`, so one invalidation
 * refreshes a whole side after a change or a realtime push.
 */
export const followKeys = {
  all: ["social", "follow"] as const,
  summary: (userId: string) => ["social", "follow", "summary", userId] as const,
  lists: ["social", "follow", "list"] as const,
  userLists: (userId: string) => ["social", "follow", "list", userId] as const,
  list: (userId: string, list: social.FollowList, query: string) =>
    ["social", "follow", "list", userId, list, query] as const,
  requests: ["social", "follow", "requests"] as const,
  requestList: (direction: social.FollowRequestDirection) =>
    ["social", "follow", "requests", direction] as const,
  incomingCount: ["social", "follow", "requests", "incoming-count"] as const,
  settings: ["social", "follow", "settings"] as const,
  // Not under `all`: following someone from the list keeps them in it (as フォロー中).
  suggestions: ["social", "follow-suggestions"] as const,
};

export const circleKeys = {
  all: ["social", "circles"] as const,
  list: ["social", "circles", "list"] as const,
  circle: (id: string) => ["social", "circles", "one", id] as const,
  members: (id: string) => ["social", "circles", "members", id] as const,
  candidates: (id: string, query: string) =>
    ["social", "circles", "candidates", id, query] as const,
};
