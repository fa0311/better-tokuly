export type BookmarksTab = "saved" | "reactions";

export interface BookmarksSearch {
  tab?: BookmarksTab;
  /** A collection's ID; absent = everything saved. */
  collection?: string;
}

/** Reads `/bookmarks?tab=…&collection=…`. */
export function parseBookmarksSearch(search: Record<string, unknown>): BookmarksSearch {
  return {
    tab: search.tab === "reactions" ? "reactions" : undefined,
    collection:
      typeof search.collection === "string" && search.collection ? search.collection : undefined,
  };
}
