/**
 * What `/search` accepts in its address. Kept free of the generated API
 * code, so a route reading the address loads none of it.
 */

/** Result orders the API offers besides relevance, by their name in an address. */
export const SORTS = ["newest", "popular"] as const;
export type Sort = (typeof SORTS)[number];

/** Longest query the API accepts. */
export const MAX_QUERY_LENGTH = 100;

/** Every kind's name in an address (`/search?kind=people`); `kinds.ts` maps them to the API's kinds. */
export const KIND_SLUGS = [
  "people",
  "channels",
  "notes",
  "videos",
  "shorts",
  "live",
  "rooms",
  "clips",
  "playlists",
  "communities",
  "threads",
  "answers",
  "stamps",
  "emoji",
  "places",
  "tags",
  "news",
] as const;
export type KindSlug = (typeof KIND_SLUGS)[number];
