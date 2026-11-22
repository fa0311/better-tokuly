import { create } from "zustand";

/** Places in the navigation that can carry an unread counter. */
export type BadgeKey = "notifications" | "chat" | "friends";

/**
 * Unread counters shown on the navigation. Each feature keeps its own entry
 * current (from its queries and realtime events) with `setBadge`; the app
 * shell only reads.
 */
export const useBadges = create<Record<BadgeKey, number>>(() => ({
  notifications: 0,
  chat: 0,
  friends: 0,
}));

export function setBadge(key: BadgeKey, count: number): void {
  if (useBadges.getState()[key] !== count) useBadges.setState({ [key]: Math.max(0, count) });
}
