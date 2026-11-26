import type { Href } from "expo-router";
import hub from "#/generated/seams/hub";

/**
 * Hub entries: the way into every area that has no tab, from the last tab
 * (the own profile, `app/(app)/(tabs)/me.tsx`).
 *
 * A feature default-exports its entries from `src/features/<area>/hub.ts`:
 *
 *   export default [
 *     { id: "settings", order: 90, href: "/settings", label: () => i18n.t("mobile:settings.title") },
 *   ] satisfies HubEntry[];
 *
 * An area without a tab loses nothing that waits for the person: an entry
 * names its count (`badge`), and the hub's tab shows the sum (`HUB_BADGE`).
 */
export interface HubEntry {
  /** Unique across the app: the area's name, or `<area>-<what>`. The button's test ID is `me-<id>`. */
  id: string;
  /** Position in the list, ascending. Leave gaps (10, 20, …); settings is 90. */
  order: number;
  href: Href;
  /** The entry's name in the current language. */
  label(): string;
  /** Key of the count to show on the entry (`#/seams/badges`): what waits behind it. */
  badge?: string;
}

/** Every entry, in display order. */
export const HUB_ENTRIES: HubEntry[] = hub
  .flat()
  .sort((a, b) => a.order - b.order || a.id.localeCompare(b.id));

/** The badge key of the hub's tab: everything that waits behind the entries, added up. */
export const HUB_BADGE = "hub";

export function waitingInHub(counts: Record<string, number>): number {
  return HUB_ENTRIES.reduce(
    (sum, entry) => sum + (entry.badge ? (counts[entry.badge] ?? 0) : 0),
    0,
  );
}
