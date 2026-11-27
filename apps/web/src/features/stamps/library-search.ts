export const LIBRARY_TABS = [
  "packs",
  "personal",
  "favorites",
  "recents",
  "purchased",
  "log",
] as const;
export type LibraryTab = (typeof LIBRARY_TABS)[number];

/** Reads the library page's address parameters. */
export function validateLibrarySearch(search: Record<string, unknown>): { tab?: LibraryTab } {
  return { tab: LIBRARY_TABS.find((tab) => tab === search.tab) };
}
