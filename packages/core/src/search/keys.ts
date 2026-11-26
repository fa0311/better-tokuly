import type { search } from "@tokuly/api";
import type { Sort } from "./query";

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
