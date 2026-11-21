import { useSyncExternalStore } from "react";

/** From this width the section list sits next to the open section (Tailwind's `lg`). */
const WIDE_QUERY = "(min-width: 1024px)";

function subscribe(onChange: () => void): () => void {
  const media = window.matchMedia(WIDE_QUERY);
  media.addEventListener("change", onChange);
  return () => media.removeEventListener("change", onChange);
}

/** Whether the settings are shown as two panes (list + section) right now. */
export function useIsWide(): boolean {
  return useSyncExternalStore(
    subscribe,
    () => window.matchMedia(WIDE_QUERY).matches,
    () => false,
  );
}
