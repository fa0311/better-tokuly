import { create } from "zustand";

/**
 * Unread counters, by key. A key is whatever the feature that owns the count
 * calls it (`"chat"`, `"notifications"`, `"ask"`); hub tiles and navigation
 * items name the key they show.
 *
 * A feature keeps its entry current in one of two ways:
 * - `setBadge("chat", n)` from its `background.tsx`, when it already listens
 *   for the events that change the count;
 * - a hook in `src/features/<area>/badges.ts` (see `#/seams/badges`), when the
 *   count is simply derived from its queries.
 */
export const useBadges = create<Record<string, number>>(() => ({}));

export function setBadge(key: string, count: number): void {
  const next = Math.max(0, count);
  if ((useBadges.getState()[key] ?? 0) !== next) useBadges.setState({ [key]: next });
}

/** The count behind a key; 0 while nothing has reported one (or no key is given). */
export function useBadge(key: string | undefined): number {
  return useBadges((badges) => (key ? (badges[key] ?? 0) : 0));
}
