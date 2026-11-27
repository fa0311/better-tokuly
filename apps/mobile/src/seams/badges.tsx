import { useEffect } from "react";
import { create } from "zustand";
import badges from "#/generated/seams/badges";
import { useAfterFirstScreen } from "./background";
import { HUB_BADGE, waitingInHub } from "./hub";

/**
 * Badges: unread counts by key, shown on the tabs that name the key.
 *
 * A feature default-exports a map of badge key to hook from
 * `src/features/<area>/badges.ts`; each hook runs for as long as someone is
 * signed in:
 *
 *   export default { chat: () => useUnread().badge } satisfies BadgeSources;
 *
 * A hook must be cheap on every screen: one query that realtime events keep
 * current, never polling. The counts load once the first screen is up
 * (`useAfterFirstScreen`).
 */
export type BadgeSources = Record<string, () => number>;

const SOURCES = badges.flatMap((sources: BadgeSources) => Object.entries(sources));

const useCounts = create<Record<string, number>>(() => ({}));

/**
 * Every count by key; a key nobody set is not there (nothing unread). The
 * hub's own key is the sum of what its entries name.
 */
export function useBadges(): Record<string, number> {
  const counts = useCounts();
  return { ...counts, [HUB_BADGE]: waitingInHub(counts) };
}

function Source({ badge, useCount }: { badge: string; useCount: () => number }) {
  const count = useCount();
  useEffect(() => useCounts.setState({ [badge]: count }), [badge, count]);
  // The count belongs to the account that produced it.
  useEffect(() => () => useCounts.setState({ [badge]: 0 }), [badge]);
  return null;
}

/** Runs every badge source; mounted once inside the signed-in layout. */
export function BadgeSourceRunner() {
  if (!useAfterFirstScreen()) return null;
  return SOURCES.map(([badge, useCount]) => (
    <Source key={badge} badge={badge} useCount={useCount} />
  ));
}
