import { type QueryClient, useMutation, useQueryClient } from "@tanstack/react-query";
import type { AnyRouter } from "@tanstack/react-router";
import {
  type InboxFilter,
  type InboxPages,
  notificationPath,
  withRead,
  withRemoved,
} from "@tokuly/core/notifications";
import { toast } from "sonner";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { applyUnread, inboxClient, rereadInbox, updateInboxLists } from "./api";
import { openTargetPath } from "./target";

/**
 * Runs a write to the inbox with its effect on the lists shown at once.
 *
 * A list that is read while the write is on its way can bring the old state
 * back. So when the server has answered, an edit that names its entries is
 * applied once more, and a sweeping one ("everything", which by then may
 * cover entries that arrived since) has the lists re-read instead. They are
 * also re-read when a reading was interrupted, or the server refused.
 */
async function writeAhead<T>(
  queryClient: QueryClient,
  edit: { apply(pages: InboxPages, filter: InboxFilter): InboxPages; sweeping?: boolean },
  write: () => Promise<T>,
): Promise<T> {
  let reread = updateInboxLists(queryClient, edit.apply);
  try {
    const result = await write();
    reread = edit.sweeping || updateInboxLists(queryClient, edit.apply) || reread;
    return result;
  } catch (error) {
    reread = true;
    throw error;
  } finally {
    if (reread) rereadInbox(queryClient);
  }
}

/** Marks entries read. */
export async function markRead(queryClient: QueryClient, ids: string[]): Promise<void> {
  const set = new Set(ids);
  const response = await writeAhead(
    queryClient,
    { apply: (pages, filter) => withRead(pages, set, filter) },
    () => inboxClient().markRead({ target: { case: "ids", value: { ids } } }),
  );
  applyUnread(queryClient, response.unread);
}

/**
 * What tapping a notification does, wherever it is shown (inbox row, toast,
 * system notification): it becomes read and the app goes to its target.
 * Returns false when the notification leads nowhere.
 */
export function openNotification(
  queryClient: QueryClient,
  router: AnyRouter,
  entry: { id: string; kind?: string; path: string; read?: boolean },
  options?: { replace?: boolean },
): boolean {
  // Reading is best effort: a failure leaves the entry unread, which the re-read lists show.
  if (entry.id && !entry.read) void markRead(queryClient, [entry.id]).catch(() => {});
  return openTargetPath(router, notificationPath(entry.kind, entry.path), options);
}

/** The inbox's write actions, with their toasts. */
export function useInboxActions() {
  const { t } = useTranslation("notifications");
  const queryClient = useQueryClient();
  const onError = (error: unknown) => toast.error(errorMessage(error));

  const read = useMutation({ mutationFn: (ids: string[]) => markRead(queryClient, ids), onError });

  const readAll = useMutation({
    /** `upToId` = the newest entry on screen, so nothing that arrives meanwhile is swept along unseen. */
    mutationFn: ({ category, upToId }: { category: string; upToId?: string }) =>
      writeAhead(
        queryClient,
        { apply: (pages, filter) => withRead(pages, "all", filter, category), sweeping: true },
        () =>
          inboxClient().markRead({
            target: upToId ? { case: "upToId", value: upToId } : { case: "all", value: true },
            category,
          }),
      ),
    onSuccess: (response) => {
      applyUnread(queryClient, response.unread);
      if (response.markedCount > 0)
        toast.success(t("toast.markedRead", { count: response.markedCount }));
    },
    onError,
  });

  const remove = useMutation({
    mutationFn: (ids: string[]) => {
      const set = new Set(ids);
      return writeAhead(queryClient, { apply: (pages) => withRemoved(pages, set) }, () =>
        inboxClient().deleteNotifications({ notificationIds: ids }),
      );
    },
    onSuccess: (response) => applyUnread(queryClient, response.unread),
    onError,
  });

  const clear = useMutation({
    mutationFn: (target: { category: string; readOnly: boolean }) =>
      writeAhead(
        queryClient,
        { apply: (pages) => withRemoved(pages, target), sweeping: true },
        () => inboxClient().clearNotifications(target),
      ),
    onSuccess: (response) => {
      applyUnread(queryClient, response.unread);
      toast.success(
        response.deletedCount > 0
          ? t("toast.cleared", { count: response.deletedCount })
          : t("toast.nothingToClear"),
      );
    },
    onError,
  });

  return { read, readAll, remove, clear };
}
