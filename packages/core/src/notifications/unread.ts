import type { notify } from "@tokuly/api";
import type { ClientSource } from "../query";
import { notifyKeys } from "./keys";

/** Unread inbox entries: in total and per category (absent = 0). The app's badges follow it. */
export interface Unread {
  total: number;
  byCategory: Record<string, number>;
}

export const NOTHING_UNREAD: Unread = { total: 0, byCategory: {} };

export function unreadOf(unread: notify.UnreadCount | undefined): Unread {
  return unread ? { total: unread.total, byCategory: { ...unread.byCategory } } : NOTHING_UNREAD;
}

/** The unread counts. Fetched once; every inbox response and realtime event then carries them. */
export function unreadQuery(client: ClientSource<typeof notify.InboxService>) {
  return {
    queryKey: notifyKeys.unread,
    queryFn: async () => unreadOf((await client().getUnreadCount({})).unread),
    staleTime: Number.POSITIVE_INFINITY,
  };
}
