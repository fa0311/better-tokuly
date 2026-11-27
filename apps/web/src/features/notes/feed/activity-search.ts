export const ACTIVITY_TABS = ["mine", "friends", "reports"] as const;
export type ActivityTab = (typeof ACTIVITY_TABS)[number];

/** Reads `/activity?tab=…`. */
export function parseActivitySearch(search: Record<string, unknown>): { tab?: ActivityTab } {
  const tab = ACTIVITY_TABS.find((value) => value === search.tab);
  return tab && tab !== "mine" ? { tab } : {};
}
