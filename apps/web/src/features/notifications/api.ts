import { type QueryClient, useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { notify } from "@tokuly/api";
import { api } from "#/lib/api";
import { type InboxFilter, type InboxPages, PAGE_SIZE } from "./inbox-cache";

export const inboxClient = () => api().service(notify.InboxService);
export const preferencesClient = () => api().service(notify.PreferencesService);
export const pushClient = () => api().service(notify.PushService);

/** Query keys of the notify domain. Everything live-updated hangs under `inbox` and `unread`. */
export const notifyKeys = {
  all: ["notify"] as const,
  inbox: ["notify", "inbox"] as const,
  inboxList: (filter: InboxFilter) => ["notify", "inbox", filter] as const,
  unread: ["notify", "unread"] as const,
  preferences: ["notify", "preferences"] as const,
  muted: ["notify", "muted"] as const,
  pushConfig: ["notify", "push", "config"] as const,
  devices: ["notify", "push", "devices"] as const,
};

/** Unread inbox entries: in total and per category (absent = 0). */
export interface Unread {
  total: number;
  byCategory: Record<string, number>;
}

const NOTHING_UNREAD: Unread = { total: 0, byCategory: {} };

function toUnread(unread: notify.UnreadCount | undefined): Unread {
  return unread ? { total: unread.total, byCategory: { ...unread.byCategory } } : NOTHING_UNREAD;
}

/**
 * Stores the unread counts the server just told us (every inbox response and
 * realtime event carries them). The navigation badge follows this cache entry.
 */
export function applyUnread(queryClient: QueryClient, unread: notify.UnreadCount | undefined): void {
  if (unread) queryClient.setQueryData<Unread>(notifyKeys.unread, toUnread(unread));
}

/** The unread counts. Fetched once; kept current by responses and realtime events. */
export function useUnread(): Unread {
  const { data } = useQuery({
    queryKey: notifyKeys.unread,
    queryFn: async () => toUnread((await inboxClient().getUnreadCount({})).unread),
    staleTime: Number.POSITIVE_INFINITY,
  });
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

/** Applies `update` to every inbox view in the cache (each has its own filter). */
export function updateInboxLists(queryClient: QueryClient, update: (pages: InboxPages, filter: InboxFilter) => InboxPages): void {
  for (const query of queryClient.getQueryCache().findAll({ queryKey: notifyKeys.inbox })) {
    const filter = query.queryKey[2] as InboxFilter;
    queryClient.setQueryData<InboxPages>(query.queryKey, (pages) => (pages ? update(pages, filter) : pages));
  }
}

/** Preferences: category switches, quiet hours, the push pause and the account's time zone. */
export function usePreferences() {
  return useQuery({
    queryKey: notifyKeys.preferences,
    queryFn: async () => (await preferencesClient().getPreferences({})).preferences,
  });
}
