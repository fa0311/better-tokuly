import type { QueryClient } from "@tanstack/query-core";
import type { chat } from "@tokuly/api";
import { patchConversation } from "./conversation-cache";
import { chatKeys } from "./keys";
import { laterSeq } from "./model";
import { leavesGap, type Timeline, withCreated, withReaction, withUpdated } from "./timeline";

/**
 * Messages in the query cache: the window of a conversation's messages
 * (`chatKeys.timeline`), and what a message changes about its conversation
 * (the list preview, the viewer's read state).
 */

export function readTimeline(
  queryClient: QueryClient,
  conversationId: string,
): Timeline | undefined {
  return queryClient.getQueryData<Timeline>(chatKeys.timeline(conversationId));
}

/** Changes a held timeline; nothing happens to one that is not held. */
export function writeTimeline(
  queryClient: QueryClient,
  conversationId: string,
  change: (timeline: Timeline) => Timeline,
): void {
  queryClient.setQueryData<Timeline>(chatKeys.timeline(conversationId), (timeline) =>
    timeline ? change(timeline) : undefined,
  );
}

/** The conversation's list preview after a message was added. */
function conversationWith(
  conversation: chat.Conversation,
  message: chat.Message,
): chat.Conversation {
  if (message.seq <= conversation.lastSeq && conversation.lastMessage?.id !== message.id)
    return conversation;
  return {
    ...conversation,
    lastMessage: message,
    lastSeq: laterSeq(message.seq, conversation.lastSeq),
    lastActivityAt: message.createdAt ?? conversation.lastActivityAt,
  };
}

/** What storing a new message found out. */
export interface Stored {
  /** The client holds the conversation (false: it is new to this client). */
  known: boolean;
  /**
   * Messages between the held window and this one were missed: the window
   * was left as it is, and the caller fetches what is missing.
   */
  missed: boolean;
}

/**
 * Stores a message that was just created — one this client sent (from the
 * response) or one announced by an event.
 */
export function storeCreated(queryClient: QueryClient, message: chat.Message): Stored {
  const timeline = readTimeline(queryClient, message.conversationId);
  const missed = Boolean(timeline && !timeline.hasNewer && leavesGap(timeline, message));
  if (!missed)
    writeTimeline(queryClient, message.conversationId, (current) => withCreated(current, message));
  const known = patchConversation(queryClient, message.conversationId, (conversation) =>
    conversationWith(conversation, message),
  );
  return { known, missed };
}

/** Stores a changed message (edited, deleted, pinned, preview arrived…). */
export function storeUpdated(
  queryClient: QueryClient,
  message: chat.Message,
  fromEvent: boolean,
): void {
  writeTimeline(queryClient, message.conversationId, (current) =>
    withUpdated(current, message, fromEvent),
  );
  patchConversation(queryClient, message.conversationId, (conversation) =>
    conversation.lastMessage?.id === message.id
      ? { ...conversation, lastMessage: message }
      : conversation,
  );
}

/** Applies one person's reaction change to the stored message. */
export function storeReaction(
  queryClient: QueryClient,
  conversationId: string,
  messageId: string,
  change: { emoji: string; added: boolean; count: number; byMe: boolean },
): void {
  writeTimeline(queryClient, conversationId, (current) => withReaction(current, messageId, change));
}

/** The viewer's side of a conversation after they read up to `readSeq`. */
export function storeOwnRead(
  queryClient: QueryClient,
  conversationId: string,
  readSeq: bigint,
  unreadCount: number,
): void {
  patchConversation(queryClient, conversationId, (conversation) => {
    const viewer = conversation.viewer;
    if (!viewer || (viewer.readSeq >= readSeq && viewer.unreadCount === unreadCount))
      return conversation;
    return {
      ...conversation,
      viewer: {
        ...viewer,
        readSeq: laterSeq(readSeq, viewer.readSeq),
        unreadCount,
        unreadMentionCount: unreadCount === 0 ? 0 : viewer.unreadMentionCount,
      },
    };
  });
}
