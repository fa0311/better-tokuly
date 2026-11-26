import {
  keepPreviousData,
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { search } from "@tokuly/api";
import { nextCursor } from "@tokuly/core/query";
import { type Kind, searchKeys, searchQueries } from "@tokuly/core/search";
import { useEffect, useState } from "react";
import { Alert } from "react-native";
import { api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";

export const searchClient = () => api().service(search.SearchService);

/** How long typing has to pause before the API is asked (the contract suggests ~150 ms). */
const TYPEAHEAD_DELAY_MS = 150;

const fail = (error: unknown) => Alert.alert(errorMessage(error, { namespace: "search" }));

/** `value`, but only after it stopped changing for `delayMs`. */
function useDebounced<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value), delayMs);
    return () => clearTimeout(timer);
  }, [value, delayMs]);
  return debounced;
}

export const useTrending = () => useQuery(searchQueries.trending(searchClient));

export const useRecentSearches = () => useQuery(searchQueries.recent(searchClient));

/**
 * Typeahead for what is typed. Each input has its own cache entry and the
 * request for superseded input is cancelled, so an old answer never shows.
 */
export function useSuggest(text: string) {
  const debounced = useDebounced(text.trim(), TYPEAHEAD_DELAY_MS);
  const kind = search.SearchKind.UNSPECIFIED;
  return useQuery({
    queryKey: searchKeys.suggest(debounced, kind),
    queryFn: ({ signal }) => searchClient().suggest({ query: debounced, kind }, { signal }),
    enabled: debounced.length > 0,
    placeholderData: keepPreviousData,
    staleTime: 30_000,
  });
}

/** A query to show the results of. */
export interface Asked {
  query: string;
  /**
   * The person asked for it themselves: it is remembered in their recent
   * searches and counts towards trending. One opened from a link is not.
   */
  remember: boolean;
  /**
   * When it was asked (0 for a link). Each asking is sent afresh, so it is
   * remembered even when the same results were shown a moment ago (opened
   * from a link, or asked before).
   */
  at: number;
}

/** The results of an asked query in one tab (`undefined`: every kind), page by page. */
export function useResults(asked: Asked, kind: Kind | undefined, enabled = true) {
  const queryClient = useQueryClient();
  const { query, remember, at } = asked;
  const tab = kind ?? search.SearchKind.UNSPECIFIED;
  return useInfiniteQuery({
    queryKey: [...searchKeys.results(query, tab, undefined, undefined), at],
    queryFn: async ({ pageParam, signal }) => {
      const response = await searchClient().search(
        {
          query,
          kind: tab,
          page: { cursor: pageParam, limit: 20 },
          skipHistory: !remember || pageParam !== "",
        },
        { signal },
      );
      if (remember && pageParam === "")
        void queryClient.invalidateQueries({ queryKey: searchKeys.recent });
      return response;
    },
    initialPageParam: "",
    getNextPageParam: nextCursor,
    enabled: enabled && query.length > 0,
    staleTime: 60_000,
  });
}

type Client = ReturnType<typeof searchClient>;
type RecentData = Awaited<ReturnType<Client["listRecentSearches"]>>;

/**
 * Forget one search, forget all of them, pause or resume the history: each
 * shows at once and is taken back when the server says no. The client is the
 * account's that tapped, taken before anything is awaited — for clearing,
 * before the confirmation is even asked (`clear()` returns what confirms it).
 */
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
    fail(error);
  };
  const settle = () => {
    // Typeahead offers recent searches too.
    void queryClient.invalidateQueries({ queryKey: searchKeys.suggestions });
    return queryClient.invalidateQueries({ queryKey: searchKeys.recent });
  };
  type Input<T> = { client: Client; value: T };
  const forget = useMutation({
    mutationFn: ({ client, value }: Input<string>) => client.deleteRecentSearch({ id: value }),
    onMutate: ({ value }) =>
      patch((data) => ({ ...data, searches: data.searches.filter((item) => item.id !== value) })),
    onError: (error, _input, context) => rollback(error, context),
    onSettled: settle,
  });
  const clear = useMutation({
    mutationFn: ({ client }: Input<null>) => client.clearRecentSearches({}),
    onMutate: () => patch((data) => ({ ...data, searches: [] })),
    onError: (error, _input, context) => rollback(error, context),
    onSettled: settle,
  });
  const setPaused = useMutation({
    mutationFn: ({ client, value }: Input<boolean>) => client.setHistoryPaused({ paused: value }),
    onMutate: ({ value }) => patch((data) => ({ ...data, historyPaused: value })),
    onError: (error, _input, context) => rollback(error, context),
    onSettled: settle,
  });
  return {
    forget: (id: string) => forget.mutate({ client: searchClient(), value: id }),
    clear: () => {
      const client = searchClient();
      return () => clear.mutate({ client, value: null });
    },
    setPaused: (paused: boolean) => setPaused.mutate({ client: searchClient(), value: paused }),
  };
}
