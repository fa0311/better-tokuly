import { type QueryClient, useMutation, useQueryClient } from "@tanstack/react-query";
import type { AnyRouter } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { errorMessage } from "#/lib/errors";
import { applyUnread, inboxClient, notifyKeys, updateInboxLists } from "./api";
import { withRead, withRemoved } from "./inbox-cache";
import { openTargetPath } from "./target";

/**
 * Marks entries read: the lists change at once, the server follows. When the
 * server refuses, the lists are re-read so they show the truth again.
 */
export async function markRead(queryClient: QueryClient, ids: string[]): Promise<void> {
  const set = new Set(ids);
  updateInboxLists(queryClient, (pages, filter) => withRead(pages, set, filter));
  try {
    const response = await inboxClient().markRead({ target: { case: "ids", value: { ids } } });
    applyUnread(queryClient, response.unread);
  } catch (error) {
    void queryClient.invalidateQueries({ queryKey: notifyKeys.inbox });
    throw error;
  }
}

/**
 * What tapping a notification does, wherever it is shown (inbox row, toast,
 * system notification): it becomes read and the app goes to its target.
 * Returns false when the notification leads nowhere.
 */
export function openNotification(queryClient: QueryClient, router: AnyRouter, entry: { id: string; path: string; read?: boolean }, options?: { replace?: boolean }): boolean {
  // Reading is best effort: a failure leaves the entry unread, which the re-read lists show.
  if (entry.id && !entry.read) void markRead(queryClient, [entry.id]).catch(() => {});
  return openTargetPath(router, entry.path, options);
}

/** The inbox's write actions, with their optimistic list edits and toasts. */
export function useInboxActions() {
  const { t } = useTranslation("notifications");
  const queryClient = useQueryClient();
  const onError = (error: unknown) => {
    toast.error(errorMessage(error));
    void queryClient.invalidateQueries({ queryKey: notifyKeys.inbox });
  };

  const read = useMutation({ mutationFn: (ids: string[]) => markRead(queryClient, ids), onError: (error) => toast.error(errorMessage(error)) });

  const readAll = useMutation({
    /** `upToId` = the newest entry on screen, so nothing that arrives meanwhile is swept along unseen. */
    mutationFn: ({ category, upToId }: { category: string; upToId?: string }) =>
      inboxClient().markRead({ target: upToId ? { case: "upToId", value: upToId } : { case: "all", value: true }, category }),
    onMutate: ({ category }) => updateInboxLists(queryClient, (pages, filter) => withRead(pages, "all", filter, category)),
    onSuccess: (response) => {
      applyUnread(queryClient, response.unread);
      if (response.markedCount > 0) toast.success(t("toast.markedRead", { count: response.markedCount }));
    },
    onError,
  });

  const remove = useMutation({
    mutationFn: (ids: string[]) => inboxClient().deleteNotifications({ notificationIds: ids }),
    onMutate: (ids) => {
      const set = new Set(ids);
      updateInboxLists(queryClient, (pages) => withRemoved(pages, set));
    },
    onSuccess: (response) => applyUnread(queryClient, response.unread),
    onError,
  });

  const clear = useMutation({
    mutationFn: (target: { category: string; readOnly: boolean }) => inboxClient().clearNotifications(target),
    onMutate: (target) => updateInboxLists(queryClient, (pages) => withRemoved(pages, target)),
    onSuccess: (response) => {
      applyUnread(queryClient, response.unread);
      toast.success(response.deletedCount > 0 ? t("toast.cleared", { count: response.deletedCount }) : t("toast.nothingToClear"));
      // Entries beyond the loaded pages are gone too; start the lists afresh.
      return queryClient.invalidateQueries({ queryKey: notifyKeys.inbox });
    },
    onError,
  });

  return { read, readAll, remove, clear };
}
