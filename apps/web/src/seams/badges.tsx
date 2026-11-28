import { useEffect, useState } from "react";
import { forgetShownBadges, setBadge, useShownBadges } from "#/stores/badges";

/**
 * Badge sources: counts that are derived from a feature's own queries.
 *
 * A feature default-exports the hook that counts the badge named after it
 * from `src/features/<area>/badges.ts`:
 *
 *   export default useAskBadge satisfies BadgeSource;
 *
 * Its file loads and the hook starts the first time a screen shows that
 * badge (`useBadge("ask")`: a hub tile), and the hook runs from then on for as
 * long as the person stays signed in; its result is what `useBadge` returns
 * everywhere. Until then nothing of it is loaded. A count every page shows
 * (chat, notifications) is set by the feature's `background.tsx` instead.
 *
 * A hook must be cheap on every page: one query that realtime events keep
 * current, never polling.
 */
export type BadgeSource = () => number;

const LOADERS = new Map(
  Object.entries(import.meta.glob<{ default: BadgeSource }>("../features/*/badges.ts")).map(
    ([path, load]) => [path.split("/")[2] ?? path, load] as const,
  ),
);

/** Sources already loaded, so a remount starts counting at once. */
const loaded = new Map<string, BadgeSource>();

/**
 * Loads one badge's source and runs it. A load that fails (the connection
 * dropped right as the badge first showed) leaves only this badge at 0 and
 * is tried again once the device is back online: never the app's error screen.
 */
function Runner({ badge }: { badge: string }) {
  const [useCount, setUseCount] = useState(() => loaded.get(badge));
  useEffect(() => {
    const load = LOADERS.get(badge);
    if (useCount || !load) return;
    let current = true;
    const attempt = () =>
      load().then(
        (module) => {
          loaded.set(badge, module.default);
          if (current) setUseCount(() => module.default);
        },
        () => {},
      );
    void attempt();
    window.addEventListener("online", attempt);
    return () => {
      current = false;
      window.removeEventListener("online", attempt);
    };
  }, [badge, useCount]);
  return useCount ? <Source badge={badge} useCount={useCount} /> : null;
}

function Source({ badge, useCount }: { badge: string; useCount: BadgeSource }) {
  const count = useCount();
  useEffect(() => setBadge(badge, count), [badge, count]);
  // The count belongs to the session that produced it.
  useEffect(() => () => setBadge(badge, 0), [badge]);
  return null;
}

/** Runs the source of every badge shown since sign-in; mounted once inside the signed-in layout. */
export function BadgeSourceRunner() {
  const shown = useShownBadges();
  useEffect(() => forgetShownBadges, []);
  return shown.map((badge) => <Runner key={badge} badge={badge} />);
}
