import {
  KIND_SLUGS,
  type KindSlug,
  MAX_QUERY_LENGTH,
  SORTS,
  type Sort,
} from "@tokuly/core/search/params";

/**
 * What `/search` shows, as kept in the address: the address is the only
 * source of truth, so a result page can be shared, reloaded and walked
 * through with back and forward.
 */
export interface SearchParams {
  /** The query; absent on the explore page. */
  q?: string;
  /** The open tab; absent for "all". */
  kind?: KindSlug;
  sort?: Sort;
  /** Only documents of this account ("search this profile"). */
  owner?: string;
}

/** A parameter as the text that was in the address: `q=2026`, `q=true` and `q=null` are queries too. */
function text(value: unknown): string {
  if (typeof value === "string") return value;
  if (value === undefined) return "";
  return typeof value === "object" && value !== null ? JSON.stringify(value) : String(value);
}

/**
 * Reads the search parameters of an address. The router hands them over as
 * parsed JSON, so a query made of digits arrives as a number.
 */
export function parseSearchParams(input: Record<string, unknown>): SearchParams {
  const q = text(input.q).trim().slice(0, MAX_QUERY_LENGTH);
  const owner = text(input.owner);
  return {
    q: q || undefined,
    kind: KIND_SLUGS.find((slug) => slug === input.kind),
    sort: SORTS.find((sort) => sort === input.sort),
    owner: owner || undefined,
  };
}
