import type { ChannelTab } from "./channels-api";

export function parseChannelsSearch(search: Record<string, unknown>): {
  tab?: ChannelTab;
  q?: string;
} {
  const tab = search.tab;
  return {
    tab: tab === "followed" || tab === "managed" ? tab : undefined,
    q: typeof search.q === "string" && search.q ? search.q : undefined,
  };
}
