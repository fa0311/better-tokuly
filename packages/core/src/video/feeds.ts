import { video } from "@tokuly/api";
import type { ClientSource } from "../query";
import { videoKeys } from "./keys";
import type { Feed } from "./search";

const { VideoCategory, TrafficSource } = video;

/** What a visit from each feed reports as where the viewer came from. */
export const FEED_SOURCES: Record<Feed, video.TrafficSource> = {
  home: TrafficSource.HOME,
  subscriptions: TrafficSource.SUBSCRIPTIONS,
  trending: TrafficSource.TRENDING,
};

/** Trending is filtered by the server; the other two feeds are filtered by the app, from what is loaded. */
export function feedKey(feed: Feed, category: video.VideoCategory) {
  return feed === "home"
    ? videoKeys.home
    : feed === "subscriptions"
      ? videoKeys.subscriptionsFeed
      : videoKeys.trending(category);
}

export interface FeedPage {
  videos: video.VideoSummary[];
  page?: { nextCursor: string };
}

/** One page of a feed. */
export function loadFeedPage(
  client: ClientSource<typeof video.VideoFeedService>,
  feed: Feed,
  category: video.VideoCategory,
  cursor: string,
  limit = 24,
): Promise<FeedPage> {
  const page = { limit, cursor };
  if (feed === "home") return client().getHomeFeed({ page });
  if (feed === "subscriptions") return client().getSubscriptionsFeed({ page });
  return client().listTrending({ category, page });
}

/** What the viewer asked not to see again, kept until they leave the feed (so it can be undone in place). */
export type Dismissal = { kind: "video" } | { kind: "channel"; channelId: string };

/**
 * The cards a feed shows from its loaded pages: each video once, of the
 * chosen genre, without the other videos of a channel the viewer turned
 * away (the dismissed card itself stays, as the way back).
 */
export function feedVideos(
  pages: readonly FeedPage[],
  feed: Feed,
  category: video.VideoCategory,
  dismissed: Readonly<Record<string, Dismissal>> = {},
): video.VideoSummary[] {
  const hiddenChannels = new Set(
    Object.values(dismissed).flatMap((entry) =>
      entry.kind === "channel" ? [entry.channelId] : [],
    ),
  );
  const seen = new Set<string>();
  return pages
    .flatMap((page) => page.videos)
    .filter((summary) => {
      if (seen.has(summary.id)) return false;
      seen.add(summary.id);
      if (
        feed !== "trending" &&
        category !== VideoCategory.UNSPECIFIED &&
        summary.category !== category
      )
        return false;
      return dismissed[summary.id] !== undefined || !hiddenChannels.has(summary.channel?.id ?? "");
    });
}
