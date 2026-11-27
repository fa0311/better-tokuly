export const PLANS_TABS = ["plans", "polls", "moments"] as const;
export type PlansTab = (typeof PLANS_TABS)[number];

/** `?tab=plans|polls|moments`; anything else is the plans tab. */
export function parsePlansSearch(search: Record<string, unknown>): { tab?: PlansTab } {
  return PLANS_TABS.includes(search.tab as PlansTab) && search.tab !== "plans"
    ? { tab: search.tab as PlansTab }
    : {};
}
