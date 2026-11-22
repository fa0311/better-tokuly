import { useEffect } from "react";
import { setBadge } from "#/stores/badges";

/**
 * Badge sources: counts that are derived from a feature's own queries.
 *
 * A feature default-exports a map of badge key to hook from
 * `src/features/<area>/badges.ts`; each hook runs for as long as someone is
 * signed in and its result is what `useBadge(key)` returns everywhere (hub
 * tiles, the navigation):
 *
 *   export default { ask: useAskBadge } satisfies BadgeSources;
 *
 * A hook must be cheap on every page: one query that realtime events keep
 * current, never polling.
 */
export type BadgeSources = Record<string, () => number>;

const modules = import.meta.glob<{ default: BadgeSources }>("../features/*/badges.ts", {
  eager: true,
});

const SOURCES = Object.values(modules).flatMap((module) => Object.entries(module.default));

function Source({ badge, useCount }: { badge: string; useCount: () => number }) {
  const count = useCount();
  useEffect(() => setBadge(badge, count), [badge, count]);
  // The count belongs to the session that produced it.
  useEffect(() => () => setBadge(badge, 0), [badge]);
  return null;
}

/** Runs every badge source; mounted once inside the signed-in layout. */
export function BadgeSourceRunner() {
  return SOURCES.map(([badge, useCount]) => (
    <Source key={badge} badge={badge} useCount={useCount} />
  ));
}
