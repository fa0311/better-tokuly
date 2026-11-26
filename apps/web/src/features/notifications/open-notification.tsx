import { useQueryClient } from "@tanstack/react-query";
import { useRouter } from "@tanstack/react-router";
import { notificationPath } from "@tokuly/core/notifications";
import { useEffect } from "react";
import { PaneSpinner } from "#/components/ui/spinner";
import { markRead } from "./inbox-actions";
import { resolveTargetPath } from "./target";

const INBOX_PATH = "/notifications";

/**
 * Where a tap on a system notification lands when the app was not open: the
 * notification becomes read and this page is replaced by its target (the
 * inbox when it has none).
 */
export function OpenNotification({
  path,
  id,
  kind,
}: {
  path: string;
  id: string;
  /** The notification's kind, when known: some kinds' paths lead elsewhere here (see `notificationPath`). */
  kind?: string;
}) {
  const queryClient = useQueryClient();
  const router = useRouter();

  useEffect(() => {
    if (id) void markRead(queryClient, [id]).catch(() => {});
    const target = resolveTargetPath(notificationPath(kind, path));
    if (target?.type === "external") {
      // No tap to open a new tab with: leave for the link in this one.
      window.location.replace(target.url);
      return;
    }
    // A target that points back here would never arrive anywhere.
    const href = target && !target.href.startsWith(`${INBOX_PATH}/open`) ? target.href : INBOX_PATH;
    void router.navigate({ href, replace: true });
  }, [queryClient, router, path, id, kind]);

  return <PaneSpinner />;
}
