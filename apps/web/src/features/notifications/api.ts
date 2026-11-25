import { useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { notify } from "@tokuly/api";
import {
  applyInboxChange,
  type InboxFilter,
  NOTHING_UNREAD,
  notifyKeys,
  PAGE_SIZE,
  rereadInbox,
  storeUnread,
  type Unread,
  unreadQuery,
  updateInboxLists,
} from "@tokuly/core/notifications";
import { api } from "#/lib/api";

// The notify keys and cache edits live in `@tokuly/core/notifications`; the folder reads them from here.
export { applyInboxChange, notifyKeys, rereadInbox, type Unread, updateInboxLists };

export const inboxClient = () => api().service(notify.InboxService);
export const preferencesClient = () => api().service(notify.PreferencesService);
export const pushClient = () => api().service(notify.PushService);

/** Stores the unread counts the server just told us; the navigation badge follows them. */
export const applyUnread = storeUnread;

/** The unread counts. Fetched once; kept current by responses and realtime events. */
export function useUnread(): Unread {
  const { data } = useQuery(unreadQuery(inboxClient));
  return data ?? NOTHING_UNREAD;
}

/** One view of the inbox (a filter tab), newest first, paged. */
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
      applyUnread(queryClient, response.unread);
      return response;
    },
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
}

/** Preferences: category switches, quiet hours, the push pause and the account's time zone. */
export function usePreferences() {
  return useQuery({
    queryKey: notifyKeys.preferences,
    queryFn: async () => (await preferencesClient().getPreferences({})).preferences,
  });
}
