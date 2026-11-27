import { type ComponentType, lazy } from "react";

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
 * arrives before its listener.
 */
const LOADERS = import.meta.glob<{ default: ComponentType }>("../features/*/background.tsx");

// This module loads with the signed-in layout: fetch them all now, in parallel with the page.
const FEATURES = Object.entries(LOADERS).map(([path, load]) => {
  const loading = load();
  return { path, Component: lazy(() => loading) };
});

/** Suspends until every background feature is loaded. */
export function BackgroundFeatures() {
  return FEATURES.map(({ path, Component }) => <Component key={path} />);
}
