import { useQueryClient } from "@tanstack/react-query";
import { useRouter } from "@tanstack/react-router";
import { useEffect } from "react";
import { realtime } from "#/lib/realtime";
import { setBadge } from "#/stores/badges";
import { useSessionStore } from "#/stores/session";
import { callClient } from "./api";
import { CallHost } from "./calls/call-host";
import { useConversationList, useUnread } from "./conversations";
import { applyChatEvent, resynchronize } from "./events";
import { useLive } from "./live";
import { setChatRouter } from "./navigation";

/**
 * Chat, for as long as someone is signed in and whatever page they are on:
 * keeps the unread badge on the navigation current, applies every realtime
 * chat event to what the app holds (so lists and open conversations are
 * live, and the next one opens instantly), rings for incoming calls and
 * hosts the call in progress.
 */
export default function ChatBackground() {
  const queryClient = useQueryClient();
  const router = useRouter();
  const unread = useUnread();
  const userId = useSessionStore((state) =>
    state.state.status === "signedIn" ? (state.state.me.user?.id ?? "") : "",
  );
  // Loaded up front: the chat tab opens without a spinner, and events have a list to update.
  const inbox = useConversationList("inbox");

  useEffect(() => {
    setBadge("chat", unread.badge);
  }, [unread.badge]);

  // `openDirectChat` navigates through the app's router.
  useEffect(() => {
    setChatRouter(router);
    return () => setChatRouter(null);
  }, [router]);

  useEffect(() => {
    if (!userId) return;
    const client = realtime();
    const offFrame = client.onFrame((frame) => {
      switch (frame.body.case) {
        case "welcome":
          // Whatever happened while the connection was down (or not up yet) is caught up here.
          resynchronize(queryClient);
          break;
        case "chat":
          applyChatEvent(queryClient, frame.body.value.event, {
            myId: userId,
            readingId: useLive.getState().readingId,
          });
          break;
        case "presence":
          useLive.getState().setOnline(frame.body.value.userId, frame.body.value.online);
          break;
        default:
          break;
      }
    });
    client.start();
    return offFrame;
  }, [queryClient, userId]);

  // A call that was already ringing or running when the app opened was announced before we listened: ask for it.
  const withCall = (inbox.data?.pages ?? [])
    .flatMap((page) => page.conversations)
    .map((conversation) => conversation.activeCallId)
    .filter(Boolean)
    .join(",");
  useEffect(() => {
    for (const callId of withCall ? withCall.split(",") : []) {
      if (useLive.getState().calls[callId]) continue;
      void callClient()
        .getCall({ callId })
        .then((response) => response.call && useLive.getState().setCall(response.call))
        .catch(() => {});
    }
  }, [withCall]);

  return <CallHost />;
}
