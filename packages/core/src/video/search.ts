import type { video } from "@tokuly/api";

/**
 * What the video pages keep in the address, and how a route reads it. Kept
 * free of the generated API code, so reading an address loads none of it.
 */

/** The three video feeds: recommendations, the channels the viewer subscribes to, what is trending. */
export const FEEDS = ["home", "subscriptions", "trending"] as const;
export type Feed = (typeof FEEDS)[number];

/** Every `video.VideoCategory` but UNSPECIFIED: GAMING (1) … OTHER (14), as `labels.ts` lists them. */
const GENRES = [
  1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14,
] as const satisfies readonly video.VideoCategory[];

export interface FeedSearch {
  feed?: Feed;
  category?: video.VideoCategory;
}

/** Reads `?feed=` and `?category=`; unknown values fall back to the home feed without a genre. */
export function validateFeedSearch(search: Record<string, unknown>): FeedSearch {
  const feed = FEEDS.find((candidate) => candidate === search.feed);
  const category = GENRES.find((genre) => genre === Number(search.category));
  return { ...(feed && feed !== "home" ? { feed } : {}), ...(category ? { category } : {}) };
}

/** What a watch link carries: `/watch/<id>?t=90&list=<playlist>&comment=<id>`. */
export interface WatchSearch {
  /** Where to start: seconds, `1:30` or `1m30s`. */
  t?: string;
  /** The playlist being played through. */
  list?: string;
  /** A comment to open the thread at. */
  comment?: string;
}

/** Reads a watch link's search; anything that is not a non-empty value is left out. */
export function validateWatchSearch(search: Record<string, unknown>): WatchSearch {
  const text = (value: unknown) =>
    typeof value === "string" && value
      ? value
      : typeof value === "number"
        ? String(value)
        : undefined;
  const result: WatchSearch = {};
  const t = text(search.t);
  const list = text(search.list);
  const comment = text(search.comment);
  if (t) result.t = t;
  if (list) result.list = list;
  if (comment) result.comment = comment;
  return result;
}
