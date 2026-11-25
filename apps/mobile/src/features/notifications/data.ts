import {
  type QueryClient,
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { notify } from "@tokuly/api";
import {
  type InboxFilter,
  type InboxPages,
  NOTHING_UNREAD,
  notifyKeys,
  PAGE_SIZE,
  rereadInbox,
  storeUnread,
  type Unread,
  unreadQuery,
  updateInboxLists,
  withRead,
} from "@tokuly/core/notifications";
import { api } from "#/lib/api";

export const inboxClient = () => api().service(notify.InboxService);

/** The unread counts: fetched once; every inbox answer and realtime event then carries them. */
export function useUnread(): Unread {
  return useQuery(unreadQuery(inboxClient)).data ?? NOTHING_UNREAD;
}

/** One view of the inbox, newest first, paged. */
export function useInbox(filter: InboxFilter) {
  const queryClient = useQueryClient();
  return useInfiniteQuery({
    queryKey: notifyKeys.inboxList(filter),
    queryFn: async ({ pageParam }) => {
      const response = await inboxClient().listNotifications({
        page: { cursor: pageParam, limit: PAGE_SIZE },
        category: filter.category,
        unreadOnly: filter.unreadOnly,
      });
      storeUnread(queryClient, response.unread);
      return response;
    },
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
}

/**
 * Marks entries read: these, or everything up to the newest one on screen
 * (`upToId`: nothing that arrives meanwhile is swept along unseen). Shown at
 * once; a list read while the write was on its way, or a refusal, has the
 * lists read again.
 */
export async function markRead(
  queryClient: QueryClient,
  target: { ids: string[] } | { upToId: string },
): Promise<void> {
  const sweeping = "upToId" in target;
  const apply = (pages: InboxPages, filter: InboxFilter) =>
    withRead(pages, sweeping ? "all" : new Set(target.ids), filter);
  let reread = updateInboxLists(queryClient, apply);
  try {
    const response = await inboxClient().markRead({
      target: sweeping
        ? { case: "upToId", value: target.upToId }
        : { case: "ids", value: { ids: target.ids } },
    });
    storeUnread(queryClient, response.unread);
    reread = sweeping || updateInboxLists(queryClient, apply) || reread;
  } catch (error) {
    reread = true;
    throw error;
  } finally {
    if (reread) rereadInbox(queryClient);
  }
}
