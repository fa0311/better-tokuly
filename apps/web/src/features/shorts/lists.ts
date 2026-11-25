import { infiniteQueryOptions } from "@tanstack/react-query";
import {
  type FeedContext,
  isSwipeOnly,
  type ListOptions,
  type ListPage,
  listKey,
  loadListPage,
  type ShortClients,
} from "@tokuly/core/shorts";
import { discoveryClient, feedClient, nextCursor, shortClient } from "./api";

export type { FeedEntry, ListOptions } from "@tokuly/core/shorts";

const clients: ShortClients = { shorts: shortClient, feed: feedClient, discovery: discoveryClient };

/**
 * The paged list behind a feed context. Grids and the swipe feed opened from
 * them ask with the same key, so a short tapped in a grid opens in the list
 * that is already loaded.
 */
export function listQuery(context: FeedContext, options: ListOptions = {}) {
  const queryKey = listKey(context, options);
  const load = (cursor: string) => loadListPage(clients, context, cursor, options);
  const swipeOnly = isSwipeOnly(context);
  // The cursor each page was asked with, by the cursor of the page after it: what pages
  // dropped from the front are read again with when the viewer scrolls back.
  const cursorKey = JSON.stringify(queryKey);
  const before = previousCursors.get(cursorKey) ?? new Map<string, string>();
  previousCursors.set(cursorKey, before);
  return infiniteQueryOptions({
    queryKey,
    queryFn: ({ pageParam }) => load(pageParam),
    initialPageParam: "",
    getNextPageParam: (last: ListPage, _all: ListPage[], lastParam: string) => {
      const next = nextCursor(last);
      if (next) before.set(next, lastParam);
      return next;
    },
    ...(swipeOnly
      ? {
          // A feed is a list being walked through: it must not rearrange itself under the viewer.
          staleTime: Number.POSITIVE_INFINITY,
          refetchOnWindowFocus: false as const,
          // And it must not grow for ever: what lies far behind is let go, and read again on the way back.
          maxPages: FEED_MAX_PAGES,
          getPreviousPageParam: (_first: ListPage, _all: ListPage[], firstParam: string) =>
            firstParam ? before.get(firstParam) : undefined,
        }
      : {}),
  });
}

/** Pages of 10 a swipe feed keeps in memory. */
export const FEED_MAX_PAGES = 5;

const previousCursors = new Map<string, Map<string, string>>();
