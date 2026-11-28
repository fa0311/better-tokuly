import { type ComponentType, use, useEffect, useState } from "react";
import { loadModule } from "#/lib/load-module";

/**
 * Feature code that runs for as long as someone is signed in, whatever page
 * they are on: keeping an unread badge current, showing a toast for a new
 * notification, ringing for an incoming call.
 *
 * A feature opts in by default-exporting a component (usually rendering
 * nothing) from `src/features/<area>/background.tsx`; it is mounted once
 * inside the signed-in layout. Each one is its own chunk, so none of them
 * is in the app's entry; the signed-in layout fetches them all at once and
 * shows the page when they are in place, so no realtime event or shortcut
 * arrives before its listener. A chunk that fails to arrive leaves only its
 * feature off until it loads (`loadModule`), never the page.
 */
const LOADERS = import.meta.glob<{ default: ComponentType }>("../features/*/background.tsx");

// This module loads with the signed-in layout: fetch them all now, in parallel with the page.
const FEATURES = Object.entries(LOADERS).map(([path, load]) => ({
  path,
  load,
  firstLoad: load().then(
    (module) => module.default,
    () => undefined,
  ),
}));

/** Suspends until every background feature has loaded or failed to. */
export function BackgroundFeatures() {
  return FEATURES.map(({ path, load, firstLoad }) => (
    <Feature key={path} load={load} firstLoad={firstLoad} />
  ));
}

type FeatureProps = (typeof FEATURES)[number];

/** One feature, once its first load settled; one that failed is loaded again (`loadModule`). */
function Feature({ load, firstLoad }: Omit<FeatureProps, "path">) {
  const [retried, setRetried] = useState<ComponentType>();
  const Component = use(firstLoad) ?? retried;
  useEffect(() => {
    if (Component) return;
    return loadModule(load, (module) => setRetried(() => module.default));
  }, [Component, load]);
  return Component ? <Component /> : null;
}
