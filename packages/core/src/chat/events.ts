import type { QueryClient } from "@tanstack/query-core";
import { chat } from "@tokuly/api";
import { reread } from "../query";
import {
  dropConversation,
  patchConversation,
  putConversation,
  readConversation,
} from "./conversation-cache";
import { chatKeys } from "./keys";
import { storeCreated, storeOwnRead, storeReaction, storeUpdated } from "./message-cache";
import { countsAsUnread, isMuted, laterSeq, listOf, mentionsUser } from "./model";
import type { MessagePager } from "./pager";
import type { Timeline } from "./timeline";
import type { UnreadTotals } from "./totals";

/**
 * Realtime chat events applied to the query cache: the lists, the open
 * conversations, their timelines and the unread totals. What an event says
 * that is not cached (who is typing, read cursors, calls, the app's own
 * outgoing messages) is for the app to keep, next to calling this.
 */

/** Who is looking, and at what. */
export interface EventContext {
  myId: string;
  /** The conversation on screen and being read right now: its new messages do not count as unread. */
  readingId: string | null;
}

/** The app's pieces the cache edits need. */
export interface ChatSync {
  totals: UnreadTotals;
  pager: MessagePager;
  /** Fetches one conversation (with whatever extras the app keeps) and stores it. */
  fetchConversation(queryClient: QueryClient, conversationId: string): Promise<unknown>;
}

/** Fetches a conversation the client does not hold (a new one, or one that was hidden). */
function learn(queryClient: QueryClient, sync: ChatSync, conversationId: string): void {
  void sync
    .fetchConversation(queryClient, conversationId)
    .then(() => sync.totals.refresh(queryClient))
    .catch(() => {});
}

function onCreated(
  queryClient: QueryClient,
  message: chat.Message,
  context: EventContext,
  sync: ChatSync,
): void {
  const mine = message.sender?.id === context.myId;
  const before = readConversation(queryClient, message.conversationId);
  if (!before) {
    learn(queryClient, sync, message.conversationId);
    return;
  }
  const stored = storeCreated(queryClient, message);
  if (stored.missed) void sync.pager.catchUp(queryClient, message.conversationId).catch(() => {});
  if (message.kind === chat.MessageKind.SYSTEM)
    void queryClient.invalidateQueries({ queryKey: chatKeys.members(message.conversationId) });

  const viewer = before.viewer;
  if (!viewer) return;
  if (mine) {
    // The sender has read everything up to their own message.
    if (viewer.unreadCount > 0) sync.totals.refresh(queryClient);
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
    learn(queryClient, sync, message.conversationId);
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
  sync.totals.adjust(queryClient, (unread) => ({
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
    default:
      break;
  }
}

function onRead(
  queryClient: QueryClient,
  event: chat.ReadReceiptChanged,
  context: EventContext,
  sync: ChatSync,
): void {
  if (event.userId !== context.myId) return;
  // Read on another device (or by sending): the badges clear here too.
  const conversation = readConversation(queryClient, event.conversationId);
  if (!conversation?.viewer || conversation.viewer.readSeq >= event.readSeq) return;
  if (event.readSeq >= conversation.lastSeq) {
    storeOwnRead(queryClient, event.conversationId, event.readSeq, 0);
    sync.totals.refresh(queryClient);
  } else {
    // Read only part of the way: how much is left is the server's to say.
    learn(queryClient, sync, event.conversationId);
  }
}

function onConversationUpdated(
  queryClient: QueryClient,
  incoming: chat.Conversation,
  sync: ChatSync,
): void {
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
      sync.totals.refresh(queryClient);
    }
    if (!held) void sync.fetchConversation(queryClient, incoming.id).catch(() => {});
  } else if (held) {
    // Shared fields only: the person's own state and the preview stay as held.
    putConversation(queryClient, {
      ...incoming,
      viewer: held.viewer,
      lastMessage: held.lastMessage,
      lastSeq: laterSeq(held.lastSeq, incoming.lastSeq),
    });
  } else {
    learn(queryClient, sync, incoming.id);
    return;
  }
  if (held && held.memberCount !== incoming.memberCount)
    void queryClient.invalidateQueries({ queryKey: chatKeys.members(incoming.id) });
}

function onCall(queryClient: QueryClient, call: chat.Call): void {
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
  if (ended) void queryClient.invalidateQueries({ queryKey: ["chat", "calls"] });
}

/** Applies one pushed `ChatEvent` to everything the query cache holds. */
export function applyChatEvent(
  queryClient: QueryClient,
  event: chat.ChatEvent["event"],
  context: EventContext,
  sync: ChatSync,
): void {
  switch (event.case) {
    case "messageCreated":
      if (event.value.message) onCreated(queryClient, event.value.message, context, sync);
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
      onRead(queryClient, event.value, context, sync);
      break;
    case "conversationUpdated":
      if (event.value.conversation)
        onConversationUpdated(queryClient, event.value.conversation, sync);
      break;
    case "conversationRemoved":
      dropConversation(queryClient, event.value.conversationId);
      sync.totals.refresh(queryClient);
      break;
    case "call":
      if (event.value.call) onCall(queryClient, event.value.call);
      break;
    default:
      break;
  }
}

/**
 * On every Welcome of the realtime connection, the first one too: events may
 * have been missed, so the lists and counts are read again (a first load in
 * flight is replaced), the timelines on screen catch up and the others are
 * forgotten (they load fresh when opened).
 */
export function resynchronize(queryClient: QueryClient, sync: ChatSync): void {
  void reread(queryClient, [chatKeys.lists, chatKeys.unread, chatKeys.conversations]);
  for (const query of queryClient.getQueryCache().findAll({ queryKey: chatKeys.timelines })) {
    const conversationId = query.queryKey[2] as string;
    const timeline = query.state.data as Timeline | undefined;
    if (query.getObserversCount() > 0 && timeline && !timeline.hasNewer)
      void sync.pager.catchUp(queryClient, conversationId).catch(() => {});
    else if (query.getObserversCount() === 0)
      queryClient.removeQueries({ queryKey: query.queryKey, exact: true });
  }
}
