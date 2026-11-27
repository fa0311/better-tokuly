import { type BoardSearch, parseBoardSearch } from "./boards-search";

export const HOME_TABS = ["boards", "events", "news", "members", "about"] as const;
export type HomeTab = (typeof HOME_TABS)[number];

export interface HomeSearch extends BoardSearch {
  tab?: HomeTab;
}

export function parseHomeSearch(search: Record<string, unknown>): HomeSearch {
  const tab = HOME_TABS.find((each) => each === search.tab);
  return { ...parseBoardSearch(search), tab: tab === "boards" ? undefined : tab };
}
