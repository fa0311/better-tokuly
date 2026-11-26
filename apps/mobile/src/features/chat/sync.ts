import { useQueryClient } from "@tanstack/react-query";
import type { chat } from "@tokuly/api";
import { applyChatEvent, resynchronize } from "@tokuly/core/chat";
import { useEffect } from "react";
import { useMe } from "#/features/session/store";
import { realtime } from "#/lib/realtime";
import { chatSync, useConversationList } from "./data";
import { useLive } from "./live";
import { clearOutbox, settle } from "./outbox";

/** What an event says that no query holds: typing, read cursors, the message this phone was sending. */
function noteLive(event: chat.ChatEvent["event"], myId: string): void {
  const live = useLive.getState();
  switch (event.case) {
    case "messageCreated": {
      const message = event.value.message;
      if (message?.clientId && message.sender?.id === myId)
        settle(message.conversationId, message.clientId);
      break;
    }
    case "readReceipt":
      live.moveCursor(event.value.conversationId, event.value.userId, event.value.readSeq);
      break;
    case "typing":
      live.setTyping(event.value.conversationId, event.value.userId, event.value.typing);
      break;
    default:
      break;
  }
}

/**
 * Chat, for as long as an account is signed in, whatever screen is open:
 * every realtime chat event goes into the cache (lists, open conversations,
 * the unread badge), and a new connection catches up on what it missed.
 * The account leaving takes its outbox and live state with it.
 */
export function useChatSync(): void {
  const queryClient = useQueryClient();
  const myId = useMe()?.user?.id ?? "";
  // Loaded up front: the chat tab opens without a spinner, and events have a list to update.
  useConversationList("inbox");
  useEffect(() => {
    if (!myId) return;
    const client = realtime();
    const off = client.onFrame((frame) => {
      if (frame.topic) return;
      if (frame.body.case === "welcome") resynchronize(queryClient, chatSync);
      else if (frame.body.case === "chat") {
        const event = frame.body.value.event;
        noteLive(event, myId);
        applyChatEvent(
          queryClient,
          event,
          { myId, readingId: useLive.getState().readingId },
          chatSync,
        );
      }
    });
    client.start();
    return () => {
      off();
      chatSync.totals.dispose();
      clearOutbox();
      useLive.getState().reset();
    };
  }, [queryClient, myId]);
}
