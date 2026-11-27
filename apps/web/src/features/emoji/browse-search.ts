export const SORTS = ["all", "popular", "new"] as const;
export type Sort = (typeof SORTS)[number];

/** What the catalogue page shows, as kept in the address: `/emoji?q=ねこ&category=どうぶつ&sort=popular`. */
export interface BrowseSearch {
  q?: string;
  /** A category's name; the empty string means "emoji without a category". */
  category?: string;
  sort?: Sort;
  /** Only the emoji credited to this user (the link on a profile's "emoji made" card). */
  author?: string;
}

export function validateBrowseSearch(search: Record<string, unknown>): BrowseSearch {
  const text = (value: unknown) =>
    typeof value === "string"
      ? value.trim().slice(0, 64)
      : typeof value === "number"
        ? String(value)
        : undefined;
  return {
    q: text(search.q) || undefined,
    category: text(search.category),
    sort: SORTS.find((sort) => sort === search.sort),
    author: text(search.author) || undefined,
  };
}
