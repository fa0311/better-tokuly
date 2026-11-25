import type { QueryClient } from "@tanstack/react-query";
import { chat } from "@tokuly/api";
import * as sync from "@tokuly/core/chat";
import { callEnded } from "./calls/session";
import { fetchConversation, unreadTotals } from "./conversations";
import { useLive } from "./live";
import { pager } from "./messages";
import { settle } from "./outbox";

export type { EventContext } from "@tokuly/core/chat";

/** What the cache edits of `@tokuly/core/chat` need from the web app. */
const chatSync: sync.ChatSync = { totals: unreadTotals, pager, fetchConversation };

/** What an event says that is not cached: the live store, the outbox, the call in progress. */
function noteLive(event: chat.ChatEvent["event"], context: sync.EventContext): void {
  const live = useLive.getState();
  switch (event.case) {
    case "messageCreated": {
      const message = event.value.message;
      if (!message) break;
      if (message.sender?.id === context.myId && message.clientId)
        settle(message.conversationId, message.clientId);
      if (message.kind === chat.MessageKind.POV_REQUEST)
        live.setOpenPov(message.conversationId, message.povRequest);
      break;
    }
    case "messageUpdated": {
      const message = event.value.message;
      if (event.value.reason === chat.MessageUpdateReason.POV && message?.povRequest)
        live.setOpenPov(message.conversationId, message.povRequest);
      break;
    }
    case "readReceipt":
      live.setCursor(event.value.conversationId, event.value.userId, event.value.readSeq);
      break;
    case "typing":
      live.setTyping(event.value.conversationId, event.value.userId, event.value.typing);
      break;
    case "presence":
      live.setPresent(event.value.conversationId, event.value.presentUserIds);
      break;
    case "call":
      if (event.value.call) live.setCall(event.value.call);
      break;
    default:
      break;
  }
}

/** Applies one pushed `ChatEvent` to everything the client holds. */
export function applyChatEvent(
  queryClient: QueryClient,
  event: chat.ChatEvent["event"],
  context: sync.EventContext,
): void {
  noteLive(event, context);
  sync.applyChatEvent(queryClient, event, context, chatSync);
  if (event.case === "call" && event.value.call?.state === chat.CallState.ENDED)
    callEnded(event.value.call.id);
}

/** After the realtime connection (re)opened: everything that may have been missed is read again. */
export function resynchronize(queryClient: QueryClient): void {
  sync.resynchronize(queryClient, chatSync);
}
