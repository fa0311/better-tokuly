import { lazy, Suspense, useEffect } from "react";
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

const RUNNERS = new Map(
  Object.entries(import.meta.glob<{ default: BadgeSource }>("../features/*/badges.ts")).map(
    ([path, load]) => {
      const badge = path.split("/")[2] ?? path;
      const Runner = lazy(() =>
        load().then((module) => ({
          default: () => <Source badge={badge} useCount={module.default} />,
        })),
      );
      return [badge, Runner] as const;
    },
  ),
);

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
  return (
    <Suspense fallback={null}>
      {shown.map((badge) => {
        const Runner = RUNNERS.get(badge);
        return Runner && <Runner key={badge} />;
      })}
    </Suspense>
  );
}
