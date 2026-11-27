import { CATEGORY_KEYS } from "@tokuly/core/community/categories";

export type DiscoverSort = "members" | "new" | "active";

export interface DiscoverSearch {
  tab?: "discover";
  q?: string;
  category?: string;
  tag?: string;
  sort?: DiscoverSort;
}

export function parseDiscoverSearch(search: Record<string, unknown>): DiscoverSearch {
  const text = (value: unknown) => (typeof value === "string" && value ? value : undefined);
  const category = text(search.category);
  const sort = text(search.sort);
  return {
    tab: search.tab === "discover" ? "discover" : undefined,
    q: text(search.q)?.slice(0, 64),
    category: CATEGORY_KEYS.find((key) => key === category),
    tag: text(search.tag),
    sort: sort === "new" || sort === "active" ? sort : undefined,
  };
}
