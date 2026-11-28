import type { QueryClient } from "@tanstack/query-core";
import type { notify } from "@tokuly/api";
import { reread } from "../query";
import type { InboxFilter, InboxPages } from "./inbox-cache";
import { notifyKeys } from "./keys";
import { type Unread, unreadOf } from "./unread";

/**
 * The inbox and its unread counts in an app's query cache: what the reader
 * does and what the realtime connection reports, applied to every view held.
 */

/**
 * Stores the unread counts the server just told us (every inbox response and
 * realtime event carries them). The app's badges follow this cache entry.
 */
export function storeUnread(
  queryClient: QueryClient,
  unread: notify.UnreadCount | undefined,
): void {
  if (!unread) return;
  queryClient.setQueryData<Unread>(notifyKeys.unread, unreadOf(unread));
  // A count request still on its way was asked before this answer; it must not replace it.
  void queryClient.cancelQueries({ queryKey: notifyKeys.unread, exact: true });
}

/**
 * Applies `update` to every inbox view in the cache (each has its own filter).
 *
 * A list request that is under way may have read the inbox before this
 * change, and its answer would undo it. Such requests are dropped; the return
 * value says whether any was, so the caller can re-read the lists once the
 * server has the change.
 */
export function updateInboxLists(
  queryClient: QueryClient,
  update: (pages: InboxPages, filter: InboxFilter) => InboxPages,
): boolean {
  let dropped = false;
  for (const query of queryClient.getQueryCache().findAll({ queryKey: notifyKeys.inbox })) {
    const filter = query.queryKey[2] as InboxFilter;
    queryClient.setQueryData<InboxPages>(query.queryKey, (pages) =>
      pages ? update(pages, filter) : pages,
    );
    if (query.state.fetchStatus === "fetching") {
      dropped = true;
      void queryClient.cancelQueries({ queryKey: query.queryKey, exact: true });
    }
  }
  return dropped;
}

/** Re-reads the inbox views: the ones on screen now, the others when they are next shown. */
export function rereadInbox(queryClient: QueryClient): void {
  void reread(queryClient, [notifyKeys.inbox]);
}

/** A change the server already has (a realtime event): applied to the lists as they are. */
export function applyInboxChange(
  queryClient: QueryClient,
  update: (pages: InboxPages, filter: InboxFilter) => InboxPages,
): void {
  if (updateInboxLists(queryClient, update)) rereadInbox(queryClient);
}
