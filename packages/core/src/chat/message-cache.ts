import type { QueryClient } from "@tanstack/query-core";
import type { chat } from "@tokuly/api";
import { type ClientSource, pushAfter, pushTo } from "../query";
import { patchConversation } from "./conversation-cache";
import { chatKeys } from "./keys";
import { leavesGap, type Timeline, withCreated, withReaction, withUpdated } from "./timeline";

/**
 * Messages in the query cache: the window of a conversation's messages
 * (`chatKeys.timeline`), and what a message changes about its conversation
 * (the list preview, the viewer's read state).
 *
 * The window is read through the journal (`docs/realtime-consistency.md`,
 * rule 2): every change to it is a push (`writeTimeline`), and every page
 * read other than the first, and every answer that carries a message, goes
 * through `pushAfter` (`MessagePager`, `storeAnswer`, `toggleReaction`). A
 * page read before a message was edited or unsent cannot bring its old copy
 * back: the change is applied again after it.
 */

export function readTimeline(
  queryClient: QueryClient,
  conversationId: string,
): Timeline | undefined {
  return queryClient.getQueryData<Timeline>(chatKeys.timeline(conversationId));
}

/**
 * Pushes a change to a held timeline, and to every read of it in flight;
 * nothing happens to one that is not held. Changes must be safe to apply
 * again (by ID).
 */
export function writeTimeline(
  queryClient: QueryClient,
  conversationId: string,
  change: (timeline: Timeline) => Timeline,
): void {
  pushTo<Timeline>(queryClient, chatKeys.timeline(conversationId), change);
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
  writeTimeline(queryClient, message.conversationId, (current) => withCreated(current, message));
  const known = patchConversation(queryClient, message.conversationId, (conversation) =>
    conversationWith(conversation, message, unseen),
  );
  return { known, missed };
}

/** The conversation's list preview after its newest message changed. */
function storePreview(queryClient: QueryClient, message: chat.Message): void {
  patchConversation(queryClient, message.conversationId, (conversation) =>
    conversation.lastMessage?.id === message.id
      ? { ...conversation, lastMessage: message }
      : conversation,
  );
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
  storePreview(queryClient, message);
}

/**
 * Stores the message a mutation answered with (edited, unsent, pinned): the
 * answer, then what was pushed while it was on its way (`pushAfter`), so a
 * newer edit or reaction stays — in the list preview too, which takes the
 * timeline's copy. Returns the answer.
 */
export async function storeAnswer<Answer extends { message?: chat.Message }>(
  queryClient: QueryClient,
  conversationId: string,
  read: () => Promise<Answer>,
): Promise<Answer> {
  let answer: Answer | undefined;
  await pushAfter<Timeline>(queryClient, chatKeys.timeline(conversationId), async () => {
    answer = await read();
    const message = answer.message;
    return message && ((timeline) => withUpdated(timeline, message, false));
  });
  const message = answer?.message;
  if (message)
    storePreview(
      queryClient,
      readTimeline(queryClient, conversationId)?.messages.find((held) => held.id === message.id) ??
        message,
    );
  return answer as Answer;
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
  // The viewer's own reaction as it will be, and as it was: absolute, so safe to apply again.
  const count = held?.count ?? 0;
  const after = { emoji, added: adding, count: count + (adding ? 1 : -1), byMe: true };
  const before = { emoji, added: !adding, count, byMe: true };
  storeReaction(queryClient, message.conversationId, message.id, after);
  try {
    await pushAfter<Timeline>(queryClient, chatKeys.timeline(message.conversationId), async () => {
      const { reactions } = adding
        ? await client().addReaction({ messageId: message.id, emoji })
        : await client().removeReaction({ messageId: message.id, emoji });
      return (timeline) => ({
        ...timeline,
        messages: timeline.messages.map((candidate) =>
          candidate.id === message.id ? { ...candidate, reactions } : candidate,
        ),
      });
    });
  } catch (error) {
    storeReaction(queryClient, message.conversationId, message.id, before);
    throw error;
  }
}
