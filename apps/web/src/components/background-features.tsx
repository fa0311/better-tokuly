import { type ComponentType, lazy, Suspense } from "react";

/**
 * Feature code that runs for as long as someone is signed in, whatever page
 * they are on: keeping an unread badge current, showing a toast for a new
 * notification, ringing for an incoming call.
 *
 * A feature opts in by default-exporting a component (usually rendering
 * nothing) from `src/features/<area>/background.tsx`; it is mounted once
 * inside the signed-in layout. Each one is its own chunk, loaded after the
 * page has rendered, so the first page never waits for them.
 */
const FEATURES = Object.entries(
  import.meta.glob<{ default: ComponentType }>("../features/*/background.tsx"),
).map(([path, load]) => ({ path, Component: lazy(load) }));

export function BackgroundFeatures() {
  return FEATURES.map(({ path, Component }) => (
    <Suspense key={path} fallback={null}>
      <Component />
    </Suspense>
  ));
}
