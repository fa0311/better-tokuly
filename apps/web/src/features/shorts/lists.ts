import { infiniteQueryOptions } from "@tanstack/react-query";
import type { common, shorts } from "@tokuly/api";
import { discoveryClient, feedClient, nextCursor, shortClient } from "./api";
import type { FeedContext } from "./model";

/** One entry of a swipe feed: the short, and what the feed says about it. */
export interface FeedEntry {
  short: shorts.Short;
  reason?: shorts.FeedReason;
  /** The hashtag that put it into the for-you feed. */
  hashtag?: string;
  /** The friend whose channel it is. */
  friend?: common.UserSummary;
  /** The viewer saw it before; it is shown again because nothing new is left. */
  repeat?: boolean;
}

interface Page {
  entries: FeedEntry[];
  page?: { nextCursor: string };
}

const fromItems = (response: { items: shorts.FeedItem[]; page?: common.PageInfo }): Page => ({
  entries: response.items.flatMap((item) =>
    item.short
      ? [
          {
            short: item.short,
            reason: item.reason,
            hashtag: item.hashtag,
            friend: item.friend,
            repeat: item.repeat,
          },
        ]
      : [],
  ),
  page: response.page,
});

const fromShorts = (response: { shorts: shorts.Short[]; page?: common.PageInfo }): Page => ({
  entries: response.shorts.map((short) => ({ short })),
  page: response.page,
});

export interface ListOptions {
  /** Following and friends feeds: leave out what the viewer has seen. */
  skipSeen?: boolean;
  /** Channel grid and hashtag page: the order (the API's enum value). */
  sort?: number;
}

/**
 * The paged list behind a feed context. Grids and the swipe feed opened from
 * them ask with the same key, so a short tapped in a grid opens in the list
 * that is already loaded.
 */
export function listQuery(context: FeedContext, options: ListOptions = {}) {
  const { skipSeen = false, sort = 0 } = options;
  const key = "key" in context ? context.key : "";
  const isFeed =
    context.kind === "forYou" || context.kind === "following" || context.kind === "friends";
  const load = (cursor: string): Promise<Page> => {
    switch (context.kind) {
      case "following":
        return feedClient()
          .getFollowingFeed({ page: { limit: 10, cursor }, skipSeen })
          .then(fromItems);
      case "friends":
        return feedClient()
          .getFriendsFeed({ page: { limit: 10, cursor }, skipSeen })
          .then(fromItems);
      case "channel":
        return discoveryClient()
          .listChannelShorts({ channelId: key, sort, page: { limit: 24, cursor } })
          .then(fromShorts);
      case "hashtag":
        return discoveryClient()
          .listHashtagShorts({ tag: key, sort, page: { limit: 24, cursor } })
          .then(fromShorts);
      case "sound":
        return discoveryClient()
          .listSoundShorts({ soundId: key, page: { limit: 24, cursor } })
          .then(fromShorts);
      case "remixes":
        return shortClient()
          .listRemixes({ shortId: key, page: { limit: 24, cursor } })
          .then(fromShorts);
      case "saved":
        return shortClient()
          .listSavedShorts({ page: { limit: 24, cursor } })
          .then(fromShorts);
      case "liked":
        return shortClient()
          .listLikedShorts({ page: { limit: 24, cursor } })
          .then(fromShorts);
      default:
        return feedClient()
          .getForYouFeed({ page: { limit: 10, cursor } })
          .then(fromItems);
    }
  };
  return infiniteQueryOptions({
    queryKey: [
      "shorts",
      isFeed || context.kind === "link" ? "feed" : "list",
      context.kind === "link" ? "forYou" : context.kind,
      key,
      sort,
      skipSeen,
    ] as const,
    queryFn: ({ pageParam }) => load(pageParam),
    initialPageParam: "",
    getNextPageParam: nextCursor,
    // A feed is a list being walked through: it must not rearrange itself under the viewer.
    ...(isFeed || context.kind === "link"
      ? { staleTime: Number.POSITIVE_INFINITY, refetchOnWindowFocus: false as const }
      : {}),
  });
}
