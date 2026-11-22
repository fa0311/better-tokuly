import { search } from "@tokuly/api";
import { type Kind, kindFromSlug, kindSlug } from "./kinds";

/** Result orders the API offers besides relevance, by their name in the address. */
export const SORTS = ["newest", "popular"] as const;
export type Sort = (typeof SORTS)[number];

/** Longest query the API accepts. */
export const MAX_QUERY_LENGTH = 100;

/**
 * What `/search` shows, as kept in the address: the address is the only
 * source of truth, so a result page can be shared, reloaded and walked
 * through with back and forward.
 */
export interface SearchParams {
  /** The query; absent on the explore page. */
  q?: string;
  /** The open tab; absent for "all". */
  kind?: string;
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
  const kind = kindFromSlug(input.kind);
  const owner = text(input.owner);
  return {
    q: q || undefined,
    kind: kind === undefined ? undefined : kindSlug(kind),
    sort: SORTS.find((sort) => sort === input.sort),
    owner: owner || undefined,
  };
}

export function sortValue(sort: Sort | undefined): search.SearchSort {
  if (sort === "newest") return search.SearchSort.NEWEST;
  if (sort === "popular") return search.SearchSort.POPULAR;
  return search.SearchSort.RELEVANCE;
}

/** The address of a result page: `/search?q=…&kind=…`. */
export function searchHref(query: string, options: { kind?: Kind; sort?: Sort; owner?: string } = {}): string {
  const params = new URLSearchParams({ q: query.trim() });
  if (options.kind !== undefined) params.set("kind", kindSlug(options.kind));
  if (options.sort) params.set("sort", options.sort);
  if (options.owner) params.set("owner", options.owner);
  return `/search?${params.toString()}`;
}

/** A query that only looks at tags starts with '#'. */
export const isTagQuery = (query: string): boolean => /^[#＃]/.test(query.trim());

export const toTagQuery = (query: string): string => `#${query.trim().replace(/^[#＃]+/, "")}`;

export const fromTagQuery = (query: string): string => query.trim().replace(/^[#＃]+/, "");
