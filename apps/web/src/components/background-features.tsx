import { Component, type ComponentType, type ReactNode } from "react";
import { onDemand } from "#/components/on-demand";

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
 * arrives before its listener. A feature whose chunk fails to arrive, or
 * that throws, is off on its own (`onDemand`, `FeatureBoundary`), never the
 * page.
 */
const LOADERS = import.meta.glob<{ default: ComponentType }>("../features/*/background.tsx");

const FEATURES = Object.entries(LOADERS).map(([path, load]) => {
  const Feature = onDemand(() => load().then((module) => module.default));
  // This module loads with the signed-in layout: fetch them all now, in parallel with the page.
  void Feature.preload();
  return { path, Feature };
});

/** Suspends until every background feature has loaded or failed to. */
export function BackgroundFeatures() {
  return FEATURES.map(({ path, Feature }) => (
    <FeatureBoundary key={path}>
      <Feature />
    </FeatureBoundary>
  ));
}

/** A feature that throws while rendering stops, alone: the page and the other features go on. */
class FeatureBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  override state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  override render() {
    return this.state.failed ? null : this.props.children;
  }
}
