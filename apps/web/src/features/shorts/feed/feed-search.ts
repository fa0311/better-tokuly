export const FEED_TABS = ["forYou", "following", "friends"] as const;
export type FeedTab = (typeof FEED_TABS)[number];

export interface FeedSearch {
  tab?: Exclude<FeedTab, "forYou">;
}

export function validateFeedSearch(search: Record<string, unknown>): FeedSearch {
  return search.tab === "following" || search.tab === "friends" ? { tab: search.tab } : {};
}
