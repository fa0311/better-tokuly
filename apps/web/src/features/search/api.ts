import { type QueryClient, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { search } from "@tokuly/api";
import { toast } from "sonner";
import { api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import type { Sort } from "./params";

export const searchClient = () => api().service(search.SearchService);
export const searchAdminClient = () => api().service(search.SearchAdminService);

/** Query keys of the search domain. Everything starts with `"search"`. */
export const searchKeys = {
  all: ["search"] as const,
  /** Every result list of a query, whatever the tab. */
  query: (query: string) => ["search", "results", query] as const,
  results: (
    query: string,
    kind: search.SearchKind,
    sort: Sort | undefined,
    owner: string | undefined,
  ) => ["search", "results", query, kind, sort ?? "relevance", owner ?? ""] as const,
  /** Every typeahead answer. */
  suggestions: ["search", "suggest"] as const,
  suggest: (text: string, kind: search.SearchKind) => ["search", "suggest", text, kind] as const,
  trending: ["search", "trending"] as const,
  recent: ["search", "recent"] as const,
  admin: {
    all: ["search", "admin"] as const,
    stats: ["search", "admin", "stats"] as const,
    blockedTerms: ["search", "admin", "blocked"] as const,
    synonyms: ["search", "admin", "synonyms"] as const,
  },
};

// ───────────────────────────── What was searched on purpose ─────────────────────────────

/**
 * Queries the person submitted themselves in this visit (typed, or picked
 * from suggestions, trends or their history). Only those are remembered in
 * their recent searches and count towards trending: a result page opened from
 * a shared link, a reload or the back button is replayed with `skip_history`.
 */
const submitted = new Set<string>();

const intentKey = (query: string) => query.trim().toLowerCase();

export function markSubmitted(query: string): void {
  submitted.add(intentKey(query));
}

export function wasSubmitted(query: string): boolean {
  return submitted.has(intentKey(query));
}

/**
 * Call when the person asks for a search (Enter, a suggestion, a trend, one
 * of their recent searches): it is remembered, and asked afresh even when the
 * same results were shown a moment ago.
 */
export function submitSearch(queryClient: QueryClient, query: string): void {
  markSubmitted(query);
  void queryClient.invalidateQueries({ queryKey: searchKeys.query(query.trim()) });
}

// ───────────────────────────── Trending ─────────────────────────────

/** What many people are searching for right now. */
export function useTrending(limit = 10) {
  return useQuery({
    queryKey: [...searchKeys.trending, limit],
    queryFn: () => searchClient().getTrending({ limit }),
    staleTime: 60_000,
  });
}

// ───────────────────────────── Recent searches ─────────────────────────────

/** The API keeps at most this many recent searches per person. */
const RECENT_LIMIT = 50;

/** The person's own recent searches (newest first) and whether their history is paused. */
export function useRecentSearches(enabled = true) {
  return useQuery({
    queryKey: searchKeys.recent,
    queryFn: () => searchClient().listRecentSearches({ page: { limit: RECENT_LIMIT } }),
    enabled,
  });
}

/** A search that was just remembered shows up in the history without waiting for the cache to go stale. */
export function refreshRecentSearches(queryClient: QueryClient): void {
  void queryClient.invalidateQueries({ queryKey: searchKeys.recent });
}

type RecentData = Awaited<ReturnType<ReturnType<typeof searchClient>["listRecentSearches"]>>;

/** Forget one search, forget all of them, pause or resume the history: each shows at once and rolls back on failure. */
export function useHistoryActions() {
  const queryClient = useQueryClient();
  const patch = async (update: (data: RecentData) => RecentData) => {
    await queryClient.cancelQueries({ queryKey: searchKeys.recent });
    const before = queryClient.getQueryData<RecentData>(searchKeys.recent);
    if (before) queryClient.setQueryData<RecentData>(searchKeys.recent, update(before));
    return { before };
  };
  const rollback = (error: unknown, context: { before?: RecentData } | undefined) => {
    if (context?.before) queryClient.setQueryData(searchKeys.recent, context.before);
    toast.error(errorMessage(error));
  };
  const settle = () => {
    // Typeahead offers recent searches too.
    void queryClient.invalidateQueries({ queryKey: searchKeys.suggestions });
    return queryClient.invalidateQueries({ queryKey: searchKeys.recent });
  };

  const forget = useMutation({
    mutationFn: (id: string) => searchClient().deleteRecentSearch({ id }),
    onMutate: (id) =>
      patch((data) => ({ ...data, searches: data.searches.filter((item) => item.id !== id) })),
    onError: (error, _id, context) => rollback(error, context),
    onSettled: settle,
  });
  const clear = useMutation({
    mutationFn: () => searchClient().clearRecentSearches({}),
    onMutate: () => patch((data) => ({ ...data, searches: [] })),
    onError: (error, _input, context) => rollback(error, context),
    onSettled: settle,
  });
  const setPaused = useMutation({
    mutationFn: (paused: boolean) => searchClient().setHistoryPaused({ paused }),
    onMutate: (paused) => patch((data) => ({ ...data, historyPaused: paused })),
    onError: (error, _paused, context) => rollback(error, context),
    onSettled: settle,
  });
  return { forget, clear, setPaused };
}
