import type { shorts } from "@tokuly/api";

/** Query keys of the shorts domain. Everything starts with `"shorts"`, so one invalidation reaches all of it. */
export const shortKeys = {
  all: ["shorts"] as const,
  short: (shortId: string) => ["shorts", "short", shortId] as const,
  feeds: ["shorts", "feed"] as const,
  lists: ["shorts", "list"] as const,
  mine: (channelId: string, status: shorts.ShortStatus) =>
    ["shorts", "list", "mine", channelId, status] as const,
  hashtag: (tag: string) => ["shorts", "hashtag", tag] as const,
  trendingHashtags: ["shorts", "hashtag", "trending"] as const,
  hashtagSuggestions: (prefix: string) => ["shorts", "hashtag", "suggest", prefix] as const,
  sound: (soundId: string) => ["shorts", "sound", soundId] as const,
  soundSuggestions: (query: string) => ["shorts", "sound", "suggest", query] as const,
  analytics: (kind: "short" | "channel", id: string, start: string, end: string) =>
    ["shorts", "analytics", kind, id, start, end] as const,
};
