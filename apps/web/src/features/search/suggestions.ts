import { search } from "@tokuly/api";

/** The groups a suggestion list is shown in. */
export type SuggestionGroup = "recent" | "trending" | "completions" | "hits" | "search";

/** One line of the suggestion list under a search field. */
export type Suggestion = { id: string; group: SuggestionGroup } & (
  | {
      /** A query to run: one of the person's own recent searches, a popular completion, or a trend. */
      type: "query";
      query: string;
      source: "recent" | "popular" | "trending";
      /** For recent searches: its ID, so it can be forgotten from the list. */
      recentId?: string;
      /** For trends: 1 = the most searched. */
      rank?: number;
    }
  /** Something to open right away: a person, a tag, a video... */
  | { type: "hit"; hit: search.SearchHit }
  /** "Search for what I typed". */
  | { type: "search"; query: string }
);

/** The list while typing: completions, then things to jump to, then "search for this". */
export function typedSuggestions(
  text: string,
  response: Pick<search.SuggestResponse, "completions" | "hits"> | undefined,
): Suggestion[] {
  const query = text.trim();
  if (!query) return [];
  const completions: Suggestion[] = (response?.completions ?? []).map((completion) => ({
    type: "query",
    group: "completions",
    id: `completion:${completion.query}`,
    query: completion.query,
    source: completion.source === search.CompletionSource.RECENT ? "recent" : "popular",
    recentId: completion.recentSearchId || undefined,
  }));
  const hits: Suggestion[] = (response?.hits ?? []).map((hit) => ({
    type: "hit",
    group: "hits",
    id: `hit:${hit.kind}:${hit.id}`,
    hit,
  }));
  return [...completions, ...hits, { type: "search", group: "search", id: "search", query }];
}

/** The list before anything is typed: the person's recent searches, then what is trending. */
export function idleSuggestions(
  recent: search.RecentSearch[],
  trending: search.TrendingQuery[],
  limits = { recent: 5, trending: 6 },
): Suggestion[] {
  const seen = new Set<string>();
  const recents: Suggestion[] = recent.slice(0, limits.recent).map((item) => {
    seen.add(item.query.toLowerCase());
    return {
      type: "query",
      group: "recent",
      id: `recent:${item.id}`,
      query: item.query,
      source: "recent",
      recentId: item.id,
    };
  });
  const trends: Suggestion[] = trending
    .filter((item) => !seen.has(item.query.toLowerCase()))
    .slice(0, limits.trending)
    .map((item) => ({
      type: "query",
      group: "trending",
      id: `trending:${item.query}`,
      query: item.query,
      source: "trending",
      rank: item.rank,
    }));
  return [...recents, ...trends];
}
