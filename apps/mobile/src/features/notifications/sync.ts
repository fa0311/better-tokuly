import { useQueryClient } from "@tanstack/react-query";
import {
  applyInboxChange,
  NOTIFY_CATEGORIES,
  notificationPath,
  notifyKeys,
  rereadInbox,
  storeUnread,
  withRead,
  withReceived,
  withRemoved,
} from "@tokuly/core/notifications";
import { router } from "expo-router";
import { useEffect } from "react";
import { useLive } from "#/features/chat/live";
import i18n from "#/lib/i18n";
import { realtime } from "#/lib/realtime";
import {
  forgetOpeningTap,
  onPushTap,
  prepareChannels,
  setIconBadge,
  showWhileOpen,
} from "#/platform/notifications";
import { markRead, useUnread } from "./data";
import { usePushRegistration } from "./push";
import { pushRoute } from "./target";

/** Taps already followed: a tap is followed once, even when the signed-in tree starts over. */
const followed = new Set<string>();

/**
 * Notifications, for as long as an account is signed in: the inbox and its
 * unread count follow the realtime connection, the icon badge follows the
 * count, this device's push registration follows the account's choice, and
 * a tap on a push opens the screen it is about.
 */
export function useNotificationsSync(): void {
  const queryClient = useQueryClient();
  const unread = useUnread();
  usePushRegistration();

  useEffect(() => {
    void setIconBadge(unread.total).catch(() => {});
  }, [unread.total]);

  useEffect(() => {
    // One Android channel per category the server sends pushes in.
    void prepareChannels(
      Object.fromEntries(
        NOTIFY_CATEGORIES.map((category) => [category, i18n.t(`mobile:push.channels.${category}`)]),
      ),
    ).catch(() => {});
    // While the app is open, a push about the conversation being read stays quiet.
    showWhileOpen((path) => {
      const reading = useLive.getState().readingId;
      return reading !== null && pushRoute(path) === `/chat/${reading}`;
    });
  }, []);

  useEffect(() => {
    const client = realtime();
    const off = client.onFrame((frame) => {
      if (frame.topic) return;
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
          if (entry) applyInboxChange(queryClient, (pages, f) => withReceived(pages, entry, f));
          break;
        }
        case "read": {
          const ids = new Set(event.value.notificationIds);
          if (event.value.bulk) rereadInbox(queryClient);
          else applyInboxChange(queryClient, (pages, f) => withRead(pages, ids, f));
          break;
        }
        case "removed": {
          const ids = new Set(event.value.notificationIds);
          if (event.value.bulk) rereadInbox(queryClient);
          else applyInboxChange(queryClient, (pages) => withRemoved(pages, ids));
          break;
        }
        default:
          return;
      }
      storeUnread(queryClient, event.value.unread);
    });
    client.start();
    return off;
  }, [queryClient]);

  // A tap on a push: the entry becomes read and its screen opens.
  useEffect(
    () =>
      onPushTap((tap) => {
        if (followed.has(tap.key)) return;
        followed.add(tap.key);
        forgetOpeningTap();
        if (tap.notificationId)
          void markRead(queryClient, { ids: [tap.notificationId] }).catch(() => {});
        router.push(pushRoute(notificationPath(tap.kind, tap.path)) as never);
      }),
    [queryClient],
  );
}
