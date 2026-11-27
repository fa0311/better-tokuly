export const NOTES_TABS = ["feed", "home", "following", "global", "mentions"] as const;
export type NotesTab = (typeof NOTES_TABS)[number];

/** Reads `/notes?tab=…`; anything else means "the default tab". */
export function parseNotesSearch(search: Record<string, unknown>): { tab?: NotesTab } {
  const tab = NOTES_TABS.find((value) => value === search.tab);
  return tab ? { tab } : {};
}
