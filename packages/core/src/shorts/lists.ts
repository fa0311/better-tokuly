import type { common, shorts } from "@tokuly/api";
import type { ClientSource } from "../query";
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

/** One page of a feed or a grid. */
export interface ListPage {
  entries: FeedEntry[];
  page?: { nextCursor: string };
}

export interface ListOptions {
  /** Following and friends feeds: leave out what the viewer has seen. */
  skipSeen?: boolean;
  /** Channel grid and hashtag page: the order (the API's enum value). */
  sort?: number;
}

/** The three shorts services a list reads from, as the app's clients. */
export interface ShortClients {
  shorts: ClientSource<typeof shorts.ShortService>;
  feed: ClientSource<typeof shorts.ShortFeedService>;
  discovery: ClientSource<typeof shorts.ShortDiscoveryService>;
}

const fromItems = (response: { items: shorts.FeedItem[]; page?: common.PageInfo }): ListPage => ({
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

const fromShorts = (response: { shorts: shorts.Short[]; page?: common.PageInfo }): ListPage => ({
  entries: response.shorts.map((short) => ({ short })),
  page: response.page,
});

/** Feeds are read 10 at a time (one swipe after another), grids 24 (a screenful of tiles). */
const FEED_PAGE = 10;
const GRID_PAGE = 24;

/** Whether a context is walked through one short at a time (a feed), rather than shown as a grid first. */
export function isSwipeOnly(context: FeedContext): boolean {
  return (
    context.kind === "forYou" ||
    context.kind === "following" ||
    context.kind === "friends" ||
    context.kind === "link"
  );
}

/** One page of the list behind a feed context, from `cursor` (`""` = the first page). */
export function loadListPage(
  clients: ShortClients,
  context: FeedContext,
  cursor: string,
  options: ListOptions = {},
): Promise<ListPage> {
  const { skipSeen = false, sort = 0 } = options;
  const key = "key" in context ? context.key : "";
  const feed = { limit: FEED_PAGE, cursor };
  const grid = { limit: GRID_PAGE, cursor };
  switch (context.kind) {
    case "following":
      return clients.feed().getFollowingFeed({ page: feed, skipSeen }).then(fromItems);
    case "friends":
      return clients.feed().getFriendsFeed({ page: feed, skipSeen }).then(fromItems);
    case "channel":
      return clients
        .discovery()
        .listChannelShorts({ channelId: key, sort, page: grid })
        .then(fromShorts);
    case "user":
      return clients.discovery().listUserShorts({ userId: key, page: grid }).then(fromShorts);
    case "hashtag":
      return clients.discovery().listHashtagShorts({ tag: key, sort, page: grid }).then(fromShorts);
    case "sound":
      return clients.discovery().listSoundShorts({ soundId: key, page: grid }).then(fromShorts);
    case "remixes":
      return clients.shorts().listRemixes({ shortId: key, page: grid }).then(fromShorts);
    case "saved":
      return clients.shorts().listSavedShorts({ page: grid }).then(fromShorts);
    case "liked":
      return clients.shorts().listLikedShorts({ page: grid }).then(fromShorts);
    default:
      return clients.feed().getForYouFeed({ page: feed }).then(fromItems);
  }
}

/**
 * The query key of the list behind a feed context. Grids and the swipe feed
 * opened from them share it, so a short tapped in a grid opens in the list
 * that is already loaded; a link plays on into the for-you feed.
 */
export function listKey(context: FeedContext, options: ListOptions = {}) {
  const { skipSeen = false, sort = 0 } = options;
  const key = "key" in context ? context.key : "";
  return [
    "shorts",
    isSwipeOnly(context) ? "feed" : "list",
    context.kind === "link" ? "forYou" : context.kind,
    key,
    sort,
    skipSeen,
  ] as const;
}
