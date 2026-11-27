export const ADMIN_TABS = ["queue", "all", "categories", "trusted", "packs"] as const;
export type AdminTab = (typeof ADMIN_TABS)[number];

/** What the staff page shows, as kept in the address: `/admin/emoji?tab=all&q=neko`. */
export interface AdminSearch {
  tab?: AdminTab;
  /** A name to look for on the "all" tab (the "manage" link on an emoji's page). */
  q?: string;
}

export function validateAdminSearch(search: Record<string, unknown>): AdminSearch {
  return {
    tab: ADMIN_TABS.find((tab) => tab === search.tab),
    q: typeof search.q === "string" ? search.q.slice(0, 64) : undefined,
  };
}
