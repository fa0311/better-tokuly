import type { QueryClient } from "@tanstack/react-query";
import { chat } from "@tokuly/api";
import { chatKeys } from "./api";
import { callEnded } from "./calls/session";
import {
  adjustUnread,
  dropConversation,
  fetchConversation,
  patchConversation,
  putConversation,
  readConversation,
  refreshUnread,
  rereadLists,
} from "./conversations";
import { useLive } from "./live";
import { catchUp, storeCreated, storeOwnRead, storeReaction, storeUpdated } from "./messages";
import { countsAsUnread, isMuted, laterSeq, listOf, mentionsUser } from "./model";
import { settle } from "./outbox";
import type { Timeline } from "./timeline";

/** Where the applier needs to know about the person and what they are looking at. */
export interface EventContext {
  myId: string;
  /** The conversation on screen and being read right now: its new messages do not count as unread. */
  readingId: string | null;
}

/** Fetches a conversation the client does not hold (a new one, or one that was hidden). */
function learn(queryClient: QueryClient, conversationId: string): void {
  void fetchConversation(queryClient, conversationId)
    .then(() => refreshUnread(queryClient))
    .catch(() => {});
}

function onCreated(queryClient: QueryClient, message: chat.Message, context: EventContext): void {
  const mine = message.sender?.id === context.myId;
  if (mine && message.clientId) settle(message.conversationId, message.clientId);

  const before = readConversation(queryClient, message.conversationId);
  if (!before || !storeCreated(queryClient, message)) {
    learn(queryClient, message.conversationId);
    return;
  }
  if (message.kind === chat.MessageKind.POV_REQUEST)
    useLive.getState().setOpenPov(message.conversationId, message.povRequest);
  if (message.kind === chat.MessageKind.SYSTEM)
    void queryClient.invalidateQueries({ queryKey: chatKeys.members(message.conversationId) });

  const viewer = before.viewer;
  if (!viewer) return;
  if (mine) {
    // The sender has read everything up to their own message.
    if (viewer.unreadCount > 0) refreshUnread(queryClient);
    storeOwnRead(queryClient, message.conversationId, message.seq, 0);
    return;
  }
  if (
    !countsAsUnread(message) ||
    message.seq <= before.lastSeq ||
    context.readingId === message.conversationId
  )
    return;
  // A hidden conversation comes back with its next message; where it lands is the server's call.
  if (viewer.hidden) {
    learn(queryClient, message.conversationId);
    return;
  }
  const mention = mentionsUser(message, context.myId);
  patchConversation(queryClient, message.conversationId, (conversation) =>
    conversation.viewer
      ? {
          ...conversation,
          viewer: {
            ...conversation.viewer,
            unreadCount: conversation.viewer.unreadCount + 1,
            unreadMentionCount: conversation.viewer.unreadMentionCount + (mention ? 1 : 0),
          },
        }
      : conversation,
  );
  const list = listOf(before);
  adjustUnread(queryClient, (unread) => ({
    ...unread,
    badge: unread.badge + (list === "inbox" && !isMuted(viewer) ? 1 : 0),
    unreadConversations:
      unread.unreadConversations + (list === "inbox" && viewer.unreadCount === 0 ? 1 : 0),
    archivedUnread: unread.archivedUnread + (list === "archived" ? 1 : 0),
  }));
}

function onUpdated(queryClient: QueryClient, event: chat.MessageUpdated): void {
  const message = event.message;
  if (!message) return;
  storeUpdated(queryClient, message, true);
  switch (event.reason) {
    case chat.MessageUpdateReason.PINNED:
    case chat.MessageUpdateReason.UNPINNED:
    case chat.MessageUpdateReason.DELETED:
      void queryClient.invalidateQueries({ queryKey: chatKeys.pinned(message.conversationId) });
      break;
    case chat.MessageUpdateReason.POV:
      if (message.povRequest)
        useLive.getState().setOpenPov(message.conversationId, message.povRequest);
      break;
    default:
      break;
  }
}

function onRead(
  queryClient: QueryClient,
  event: chat.ReadReceiptChanged,
  context: EventContext,
): void {
  useLive.getState().setCursor(event.conversationId, event.userId, event.readSeq);
  if (event.userId !== context.myId) return;
  // Read on another device (or by sending): the badges clear here too.
  const conversation = readConversation(queryClient, event.conversationId);
  if (!conversation?.viewer || conversation.viewer.readSeq >= event.readSeq) return;
  if (event.readSeq >= conversation.lastSeq) {
    storeOwnRead(queryClient, event.conversationId, event.readSeq, 0);
    refreshUnread(queryClient);
  } else {
    // Read only part of the way: how much is left is the server's to say.
    learn(queryClient, event.conversationId);
  }
}

function onConversationUpdated(queryClient: QueryClient, incoming: chat.Conversation): void {
  const held = readConversation(queryClient, incoming.id);
  if (incoming.viewer) {
    // Built for this person: the whole truth, including their own state and the preview.
    putConversation(queryClient, incoming);
    if (
      !held ||
      held.viewer?.requestState !== incoming.viewer.requestState ||
      held.viewer?.archived !== incoming.viewer.archived ||
      held.viewer?.muted !== incoming.viewer.muted
    ) {
      refreshUnread(queryClient);
    }
    if (!held) void fetchConversation(queryClient, incoming.id).catch(() => {});
  } else if (held) {
    // Shared fields only: the person's own state and the preview stay as held.
    putConversation(queryClient, {
      ...incoming,
      viewer: held.viewer,
      lastMessage: held.lastMessage,
      lastSeq: laterSeq(held.lastSeq, incoming.lastSeq),
    });
  } else {
    learn(queryClient, incoming.id);
    return;
  }
  if (held && held.memberCount !== incoming.memberCount)
    void queryClient.invalidateQueries({ queryKey: chatKeys.members(incoming.id) });
}

function onCall(queryClient: QueryClient, call: chat.Call): void {
  useLive.getState().setCall(call);
  const ended = call.state === chat.CallState.ENDED;
  patchConversation(queryClient, call.conversationId, (conversation) => {
    const activeCallId = ended
      ? conversation.activeCallId === call.id
        ? ""
        : conversation.activeCallId
      : call.id;
    return activeCallId === conversation.activeCallId
      ? conversation
      : { ...conversation, activeCallId };
  });
  if (ended) {
    callEnded(call.id);
    void queryClient.invalidateQueries({ queryKey: ["chat", "calls"] });
  }
}

/** Applies one pushed `ChatEvent` to everything the client holds. */
export function applyChatEvent(
  queryClient: QueryClient,
  event: chat.ChatEvent["event"],
  context: EventContext,
): void {
  switch (event.case) {
    case "messageCreated":
      if (event.value.message) onCreated(queryClient, event.value.message, context);
      break;
    case "messageUpdated":
      onUpdated(queryClient, event.value);
      break;
    case "reactionChanged":
      storeReaction(queryClient, event.value.conversationId, event.value.messageId, {
        emoji: event.value.emoji,
        added: event.value.added,
        count: event.value.count,
        byMe: event.value.userId === context.myId,
      });
      void queryClient.invalidateQueries({
        queryKey: ["chat", "reactions", event.value.messageId],
      });
      break;
    case "readReceipt":
      onRead(queryClient, event.value, context);
      break;
    case "typing":
      useLive
        .getState()
        .setTyping(event.value.conversationId, event.value.userId, event.value.typing);
      break;
    case "presence":
      useLive.getState().setPresent(event.value.conversationId, event.value.presentUserIds);
      break;
    case "conversationUpdated":
      if (event.value.conversation) onConversationUpdated(queryClient, event.value.conversation);
      break;
    case "conversationRemoved":
      dropConversation(queryClient, event.value.conversationId);
      refreshUnread(queryClient);
      break;
    case "call":
      if (event.value.call) onCall(queryClient, event.value.call);
      break;
    default:
      break;
  }
}

/**
 * After the realtime connection (re)opened: events may have been missed, so
 * the lists and counts are read again, the timelines on screen catch up and
 * the others are forgotten (they load fresh when opened).
 */
export function resynchronize(queryClient: QueryClient): void {
  rereadLists(queryClient);
  refreshUnread(queryClient);
  void queryClient.invalidateQueries({ queryKey: chatKeys.conversations });
  for (const query of queryClient.getQueryCache().findAll({ queryKey: chatKeys.timelines })) {
    const conversationId = query.queryKey[2] as string;
    const timeline = query.state.data as Timeline | undefined;
    if (query.getObserversCount() > 0 && timeline && !timeline.hasNewer)
      void catchUp(queryClient, conversationId).catch(() => {});
    else if (query.getObserversCount() === 0)
      queryClient.removeQueries({ queryKey: query.queryKey, exact: true });
  }
}
