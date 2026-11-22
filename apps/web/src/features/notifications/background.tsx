import { useQueryClient } from "@tanstack/react-query";
import { useRouter } from "@tanstack/react-router";
import type { notify } from "@tokuly/api";
import { X } from "lucide-react";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { realtime } from "#/lib/realtime";
import { setBadge } from "#/stores/badges";
import { useSessionStore } from "#/stores/session";
import { applyInboxChange, applyUnread, notifyKeys, rereadInbox, useUnread } from "./api";
import { openNotification } from "./inbox-actions";
import { withRead, withReceived, withRemoved } from "./inbox-cache";
import { NotificationFace } from "./notification-face";
import { closeShownNotifications, refreshPushRegistration, serviceWorker } from "./push";

const INBOX_PATH = "/notifications";

/**
 * Runs for as long as someone is signed in: keeps the unread badge and the
 * inbox lists current from the realtime connection, announces new
 * notifications with a toast, and looks after the service worker (system
 * notification clicks, the push registration).
 */
export default function NotificationsBackground() {
  const queryClient = useQueryClient();
  const router = useRouter();
  const unread = useUnread();
  const userId = useSessionStore((s) =>
    s.state.status === "signedIn" ? (s.state.me.user?.id ?? "") : "",
  );
  const sessionId = useSessionStore((s) =>
    s.state.status === "signedIn" ? s.state.sessionId : "",
  );

  // The badge on the navigation follows the unread count (the installed app's icon takes it from there).
  useEffect(() => {
    setBadge("notifications", unread.total);
  }, [unread.total]);

  useEffect(() => {
    const client = realtime();

    const announce = (entry: notify.Notification) => {
      const open = () => {
        toast.dismiss(toastId);
        openNotification(queryClient, router, entry);
      };
      // One toast per entry: a grouped entry that grows updates its toast instead of stacking.
      const toastId = toast.custom(
        () => (
          <NotificationToast entry={entry} onOpen={open} onClose={() => toast.dismiss(toastId)} />
        ),
        {
          id: `notify:${entry.id}`,
          duration: entry.urgent ? 20_000 : 6_000,
        },
      );
    };

    const offFrame = client.onFrame((frame) => {
      if (frame.body.case === "welcome") {
        // Whatever was sent while the connection was down (or not up yet) is caught up here.
        void queryClient.invalidateQueries({ queryKey: notifyKeys.unread });
        rereadInbox(queryClient);
        return;
      }
      if (frame.body.case !== "notify") return;
      const event = frame.body.value.event;
      switch (event.case) {
        case "received": {
          const entry = event.value.notification;
          if (!entry) break;
          applyInboxChange(queryClient, (pages, filter) => withReceived(pages, entry, filter));
          if (shouldAnnounce(entry, router.state.location.pathname)) announce(entry);
          break;
        }
        case "read": {
          if (event.value.bulk) rereadInbox(queryClient);
          else {
            const ids = new Set(event.value.notificationIds);
            applyInboxChange(queryClient, (pages, filter) => withRead(pages, ids, filter));
          }
          void closeShownNotifications(event.value.bulk ? undefined : event.value.notificationIds);
          break;
        }
        case "removed": {
          if (event.value.bulk) rereadInbox(queryClient);
          else {
            const ids = new Set(event.value.notificationIds);
            applyInboxChange(queryClient, (pages) => withRemoved(pages, ids));
          }
          void closeShownNotifications(event.value.bulk ? undefined : event.value.notificationIds);
          break;
        }
        default:
          return;
      }
      applyUnread(queryClient, event.value.unread);
    });
    client.start();

    return () => {
      offFrame();
      // Signed out: nothing of this account's inbox may greet the next person.
      // (Checked, because development double-mounts effects while still signed in.)
      if (useSessionStore.getState().state.status === "signedIn") return;
      queryClient.removeQueries({ queryKey: notifyKeys.all });
      setBadge("notifications", 0);
    };
  }, [queryClient, router]);

  // A tap on a system notification, relayed by the service worker.
  useEffect(() => {
    if (!("serviceWorker" in navigator)) return;
    void serviceWorker();
    const onMessage = (event: MessageEvent) => {
      const message = event.data as { type?: string; id?: string; path?: string } | null;
      if (message?.type === "tokuly:notification-click") {
        event.ports[0]?.postMessage("ok");
        openNotification(queryClient, router, {
          id: message.id ?? "",
          path: message.path || INBOX_PATH,
        });
      } else if (message?.type === "tokuly:push-subscription-change" && userId) {
        void refreshPushRegistration({ userId, sessionId });
      }
    };
    navigator.serviceWorker.addEventListener("message", onMessage);
    return () => navigator.serviceWorker.removeEventListener("message", onMessage);
  }, [queryClient, router, userId, sessionId]);

  // Push registrations belong to a session: renew this browser's when a new one starts.
  useEffect(() => {
    if (userId) void refreshPushRegistration({ userId, sessionId });
  }, [userId, sessionId]);

  return null;
}

/**
 * Whether a new notification deserves a toast: not while the inbox itself is
 * on screen (the entry appears there), not for something already read on
 * another device, and not while nobody is looking (push covers that).
 *
 * `NotificationReceived` has no "silent" marker yet; when the contract gains
 * one, this is where it is honoured.
 */
function shouldAnnounce(entry: notify.Notification, pathname: string): boolean {
  if (entry.read || document.visibilityState !== "visible") return false;
  return pathname.replace(/\/+$/, "") !== INBOX_PATH;
}

function NotificationToast({
  entry,
  onOpen,
  onClose,
}: {
  entry: notify.Notification;
  onOpen(): void;
  onClose(): void;
}) {
  const { t } = useTranslation("notifications");
  return (
    <div className="flex w-full items-start gap-1 p-2">
      <button
        type="button"
        onClick={onOpen}
        className="flex min-w-0 flex-1 items-start gap-3 rounded-xl p-1.5 text-left"
      >
        <NotificationFace entry={entry} />
        <span className="min-w-0 flex-1">
          <span className="block truncate font-black">{entry.title}</span>
          {entry.body && (
            <span className="line-clamp-2 text-sm font-medium text-ink-soft">{entry.body}</span>
          )}
        </span>
      </button>
      <button
        type="button"
        aria-label={t("toast.dismiss")}
        onClick={onClose}
        className="grid size-8 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-2"
      >
        <X className="size-4" />
      </button>
    </div>
  );
}
