import type { ComponentType } from "react";

/**
 * Feature code that runs for as long as someone is signed in, whatever page
 * they are on: keeping an unread badge current, showing a toast for a new
 * notification, ringing for an incoming call.
 *
 * A feature opts in by default-exporting a component (usually rendering
 * nothing) from `src/features/<area>/background.tsx`; it is mounted once
 * inside the signed-in layout.
 */
const modules = import.meta.glob<{ default: ComponentType }>("../features/*/background.tsx", {
  eager: true,
});

export function BackgroundFeatures() {
  return Object.entries(modules).map(([path, module]) => <module.default key={path} />);
}
