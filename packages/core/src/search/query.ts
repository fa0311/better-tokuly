import { search } from "@tokuly/api";
import type { Sort } from "./params";

export function sortValue(sort: Sort | undefined): search.SearchSort {
  if (sort === "newest") return search.SearchSort.NEWEST;
  if (sort === "popular") return search.SearchSort.POPULAR;
  return search.SearchSort.RELEVANCE;
}

/** A query that only looks at tags starts with '#'. */
export const isTagQuery = (query: string): boolean => /^[#＃]/.test(query.trim());

export const toTagQuery = (query: string): string => `#${query.trim().replace(/^[#＃]+/, "")}`;

export const fromTagQuery = (query: string): string => query.trim().replace(/^[#＃]+/, "");
