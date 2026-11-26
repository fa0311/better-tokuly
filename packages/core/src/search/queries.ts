import type { search } from "@tokuly/api";
import type { ClientSource } from "../query";
import { searchKeys } from "./keys";

type Search = ClientSource<typeof search.SearchService>;

/** The API keeps at most this many recent searches per person. */
const RECENT_LIMIT = 50;

/** Query options of search. Each app adds its hooks around them. */
export const searchQueries = {
  /** What many people are searching for right now. */
  trending: (client: Search, limit = 10) => ({
    queryKey: [...searchKeys.trending, limit] as const,
    queryFn: () => client().getTrending({ limit }),
    staleTime: 60_000,
  }),

  /** The viewer's own recent searches (newest first) and whether their history is paused. */
  recent: (client: Search) => ({
    queryKey: searchKeys.recent,
    queryFn: () => client().listRecentSearches({ page: { limit: RECENT_LIMIT } }),
  }),
};
