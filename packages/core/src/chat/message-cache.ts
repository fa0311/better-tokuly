import type { QueryClient } from "@tanstack/query-core";
import type { chat } from "@tokuly/api";
import type { ClientSource } from "../query";
import { patchConversation } from "./conversation-cache";
import { chatKeys } from "./keys";
import {
  leavesGap,
  reactionsAfter,
  type Timeline,
  withCreated,
  withReaction,
  withUpdated,
} from "./timeline";

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

/** What a new message adds to the viewer's unread counts (an event from someone else). */
export interface Unseen {
  /** It mentions the viewer. */
  mention: boolean;
}

/**
 * The conversation after a message was added: its list preview and, for a
 * message the viewer has not seen, the unread counts. A copy that already
 * has the message (`lastSeq`) only takes its newer version.
 */
function conversationWith(
  conversation: chat.Conversation,
  message: chat.Message,
  unseen: Unseen | undefined,
): chat.Conversation {
  if (message.seq <= conversation.lastSeq)
    return conversation.lastMessage?.id === message.id
      ? { ...conversation, lastMessage: message }
      : conversation;
  const viewer = conversation.viewer;
  return {
    ...conversation,
    lastMessage: message,
    lastSeq: message.seq,
    lastActivityAt: message.createdAt ?? conversation.lastActivityAt,
    viewer:
      viewer && unseen
        ? {
            ...viewer,
            unreadCount: viewer.unreadCount + 1,
            unreadMentionCount: viewer.unreadMentionCount + (unseen.mention ? 1 : 0),
          }
        : viewer,
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
 * response) or one announced by an event; `unseen` when it adds to the
 * viewer's unread counts.
 */
export function storeCreated(
  queryClient: QueryClient,
  message: chat.Message,
  unseen?: Unseen,
): Stored {
  const timeline = readTimeline(queryClient, message.conversationId);
  const missed = Boolean(timeline && !timeline.hasNewer && leavesGap(timeline, message));
  if (!missed)
    writeTimeline(queryClient, message.conversationId, (current) => withCreated(current, message));
  const known = patchConversation(queryClient, message.conversationId, (conversation) =>
    conversationWith(conversation, message, unseen),
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
    // A copy read after this read (or a later one) knows better.
    if (!viewer || viewer.readSeq >= readSeq) return conversation;
    return {
      ...conversation,
      viewer: {
        ...viewer,
        readSeq,
        unreadCount,
        unreadMentionCount: unreadCount === 0 ? 0 : viewer.unreadMentionCount,
      },
    };
  });
}

/** Adds or removes the viewer's reaction, showing it at once and settling on the server's answer. */
export async function toggleReaction(
  queryClient: QueryClient,
  client: ClientSource<typeof chat.MessageService>,
  message: chat.Message,
  emoji: string,
): Promise<void> {
  const held = message.reactions.find((reaction) => reaction.emoji === emoji);
  const adding = !held?.reactedByMe;
  const before = message.reactions;
  const set = (reactions: chat.ReactionCount[]) =>
    writeTimeline(queryClient, message.conversationId, (current) => ({
      ...current,
      messages: current.messages.map((candidate) =>
        candidate.id === message.id ? { ...candidate, reactions } : candidate,
      ),
    }));
  set(
    reactionsAfter(before, {
      emoji,
      added: adding,
      count: (held?.count ?? 0) + (adding ? 1 : -1),
      byMe: true,
    }),
  );
  try {
    const response = adding
      ? await client().addReaction({ messageId: message.id, emoji })
      : await client().removeReaction({ messageId: message.id, emoji });
    set(response.reactions);
  } catch (error) {
    set(before);
    throw error;
  }
}
