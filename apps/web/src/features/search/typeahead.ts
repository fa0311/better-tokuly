import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { search } from "@tokuly/api";
import { searchClient, searchKeys, useRecentSearches, useTrending } from "./api";
import { useDebounced } from "./hooks";
import { idleSuggestions, typedSuggestions } from "./suggestions";

/** How long typing has to pause before the API is asked (the contract suggests ~150 ms). */
const TYPEAHEAD_DELAY_MS = 150;

/**
 * Suggestions for a search field: typeahead while there is text, recent
 * searches and trends before. A response to superseded input is never shown:
 * each input has its own cache entry and the request for the old one is
 * cancelled.
 */
export function useSuggestions(text: string, options: { kind?: search.SearchKind; /** Offer recent searches and trends while the field is empty. */ idle?: boolean; enabled?: boolean } = {}) {
  const { kind = search.SearchKind.UNSPECIFIED, idle = true, enabled = true } = options;
  const typed = text.trim();
  const debounced = useDebounced(typed, TYPEAHEAD_DELAY_MS);
  const suggest = useQuery({
    queryKey: searchKeys.suggest(debounced, kind),
    queryFn: ({ signal }) => searchClient().suggest({ query: debounced, kind }, { signal }),
    enabled: enabled && debounced.length > 0,
    placeholderData: keepPreviousData,
    staleTime: 30_000,
  });
  const wantsIdle = enabled && idle && typed.length === 0;
  const recent = useRecentSearches(wantsIdle);
  const trending = useTrending();

  const suggestions = typed ? typedSuggestions(typed, suggest.data) : wantsIdle ? idleSuggestions(recent.data?.searches ?? [], trending.data?.queries ?? []) : [];
  return {
    suggestions,
    /** The typeahead has not answered for the current text yet. */
    loading: typed.length > 0 && (typed !== debounced || suggest.isFetching),
  };
}
