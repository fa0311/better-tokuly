import type { QueryClient } from "@tanstack/query-core";
import type { chat } from "@tokuly/api";
import { chatKeys } from "./keys";

/** Chat's unread totals: the badge, and the counts of the three lists. */
export const NO_UNREAD = {
  badge: 0,
  unreadConversations: 0,
  pendingRequests: 0,
  archivedUnread: 0,
};
export type Unread = typeof NO_UNREAD;

export function unreadOf(unread: chat.UnreadCounts | undefined): Unread {
  return unread
    ? {
        badge: unread.badge,
        unreadConversations: unread.unreadConversations,
        pendingRequests: unread.pendingRequests,
        archivedUnread: unread.archivedUnread,
      }
    : NO_UNREAD;
}

/**
 * Stores the unread totals the server just reported. Whether the answer is
 * still current (no realtime adjustment happened while it was asked for) is
 * for the caller to know.
 */
export function storeUnread(queryClient: QueryClient, unread: chat.UnreadCounts): void {
  queryClient.setQueryData<Unread>(chatKeys.unread, unreadOf(unread));
  // A count request still under way was asked before this answer and must not replace it.
  void queryClient.cancelQueries({ queryKey: chatKeys.unread, exact: true });
}

/**
 * Adjusts the held unread totals for something that just happened (a
 * realtime event), never below zero. Returns false when no totals are held.
 */
export function adjustUnread(
  queryClient: QueryClient,
  change: (unread: Unread) => Unread,
): boolean {
  let adjusted = false;
  queryClient.setQueryData<Unread>(chatKeys.unread, (unread) => {
    if (!unread) return unread;
    adjusted = true;
    const next = change(unread);
    return {
      badge: Math.max(0, next.badge),
      unreadConversations: Math.max(0, next.unreadConversations),
      pendingRequests: Math.max(0, next.pendingRequests),
      archivedUnread: Math.max(0, next.archivedUnread),
    };
  });
  return adjusted;
}
