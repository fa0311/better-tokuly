import { type QueryClient, useQuery, useQueryClient } from "@tanstack/react-query";
import type { chat } from "@tokuly/api";
import {
  laterSeq,
  leavesGap,
  reactionsAfter,
  type Timeline,
  timelineOf,
  withCreated,
  withNewer,
  withOlder,
  withReaction,
  withUpdated,
} from "@tokuly/core/chat";
import { chatKeys, conversationClient, messageClient } from "./api";
import { applyUnread, patchConversation, unreadVersion } from "./conversations";
import { useLive } from "./live";

/** Messages fetched per request. */
const PAGE = 40;

type Position = chat.ListMessagesRequest["position"];

async function fetchPage(
  conversationId: string,
  position: Position,
): Promise<chat.ListMessagesResponse> {
  const response = await messageClient().listMessages({ conversationId, limit: PAGE, position });
  useLive.getState().setCursors(conversationId, response.readCursors);
  return response;
}

export function readTimeline(
  queryClient: QueryClient,
  conversationId: string,
): Timeline | undefined {
  return queryClient.getQueryData<Timeline>(chatKeys.timeline(conversationId));
}

function writeTimeline(
  queryClient: QueryClient,
  conversationId: string,
  change: (timeline: Timeline) => Timeline,
): void {
  queryClient.setQueryData<Timeline>(chatKeys.timeline(conversationId), (timeline) =>
    timeline ? change(timeline) : undefined,
  );
}

/**
 * The messages of a conversation. The first load opens on `around` when
 * given (a search hit, a notification), else on the first unread message,
 * else at the newest. After that the timeline is kept current by the realtime
 * events and grown by `loadOlder` / `loadNewer`; it is never refetched whole.
 */
export function useTimeline(conversationId: string, around?: string) {
  return useQuery({
    queryKey: chatKeys.timeline(conversationId),
    queryFn: async () => {
      const position: Position = around
        ? { case: "aroundMessageId", value: around }
        : { case: "aroundUnread", value: true };
      try {
        return timelineOf(await fetchPage(conversationId, position));
      } catch (error) {
        // The message to open on is gone (deleted, or from before the viewer joined): open normally.
        if (!around) throw error;
        return timelineOf(await fetchPage(conversationId, { case: "aroundUnread", value: true }));
      }
    },
    staleTime: Number.POSITIVE_INFINITY,
    gcTime: 10 * 60_000,
  });
}

/** Requests under way, so scrolling does not start the same one twice. */
const loading = new Set<string>();

async function once(key: string, work: () => Promise<void>): Promise<void> {
  if (loading.has(key)) return;
  loading.add(key);
  try {
    await work();
  } finally {
    loading.delete(key);
  }
}

/** Loads the page before the oldest message held. */
export function loadOlder(queryClient: QueryClient, conversationId: string): Promise<void> {
  return once(`older:${conversationId}`, async () => {
    const timeline = readTimeline(queryClient, conversationId);
    if (!timeline?.hasOlder) return;
    const page = await fetchPage(conversationId, { case: "before", value: timeline.olderCursor });
    writeTimeline(queryClient, conversationId, (current) => withOlder(current, page));
  });
}

/** Loads the page after the newest message held (on the way back from the history, or to catch up). */
export function loadNewer(queryClient: QueryClient, conversationId: string): Promise<void> {
  return once(`newer:${conversationId}`, async () => {
    const timeline = readTimeline(queryClient, conversationId);
    if (!timeline) return;
    const page = await fetchPage(conversationId, { case: "after", value: timeline.newerCursor });
    writeTimeline(queryClient, conversationId, (current) => withNewer(current, page));
  });
}

/**
 * Brings a timeline at the live end up to date after events may have been
 * missed. When too much happened meanwhile, it starts over at the newest
 * messages instead of paging through everything.
 */
export async function catchUp(queryClient: QueryClient, conversationId: string): Promise<void> {
  const timeline = readTimeline(queryClient, conversationId);
  if (!timeline || timeline.hasNewer) return;
  await loadNewer(queryClient, conversationId);
  if (readTimeline(queryClient, conversationId)?.hasNewer)
    await showNewest(queryClient, conversationId);
}

/** Replaces the timeline with the newest messages ("jump to latest"). */
export function showNewest(queryClient: QueryClient, conversationId: string): Promise<void> {
  return once(`newest:${conversationId}`, async () => {
    const page = await fetchPage(conversationId, { case: undefined });
    queryClient.setQueryData<Timeline>(chatKeys.timeline(conversationId), timelineOf(page));
  });
}

/**
 * Makes sure a message is in the timeline (a reply's original, a search hit,
 * a pin): when it is not held, the timeline is replaced by the window around
 * it. Rejects with MESSAGE_NOT_FOUND when the viewer cannot see it.
 */
export async function showMessage(
  queryClient: QueryClient,
  conversationId: string,
  messageId: string,
): Promise<void> {
  if (
    readTimeline(queryClient, conversationId)?.messages.some((message) => message.id === messageId)
  )
    return;
  const page = await fetchPage(conversationId, { case: "aroundMessageId", value: messageId });
  queryClient.setQueryData<Timeline>(chatKeys.timeline(conversationId), timelineOf(page));
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

/**
 * Stores a message that was just created — one this client sent (from the
 * response) or one announced by an event. Returns false when the client does
 * not know the conversation yet.
 */
export function storeCreated(queryClient: QueryClient, message: chat.Message): boolean {
  const timeline = readTimeline(queryClient, message.conversationId);
  if (timeline && !timeline.hasNewer && leavesGap(timeline, message)) {
    // Something in between was missed: fetch it rather than leave a hole.
    void catchUp(queryClient, message.conversationId);
  } else {
    writeTimeline(queryClient, message.conversationId, (current) => withCreated(current, message));
  }
  return patchConversation(queryClient, message.conversationId, (conversation) =>
    conversationWith(conversation, message),
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

/**
 * Marks the conversation read: everything, or up to `messageId`. The unread
 * badge follows from the answer.
 */
export async function markRead(
  queryClient: QueryClient,
  conversationId: string,
  messageId = "",
): Promise<void> {
  const askedAt = unreadVersion();
  const response = await conversationClient().markRead({ conversationId, messageId });
  storeOwnRead(queryClient, conversationId, response.readSeq, response.unreadCount);
  applyUnread(queryClient, response.unread, askedAt);
}

/** Adds or removes the viewer's reaction, showing it at once and settling on the server's answer. */
export async function toggleReaction(
  queryClient: QueryClient,
  message: chat.Message,
  emoji: string,
): Promise<void> {
  const held = message.reactions.find((reaction) => reaction.emoji === emoji);
  const adding = !held?.reactedByMe;
  const before = message.reactions;
  const optimistic = reactionsAfter(before, {
    emoji,
    added: adding,
    count: (held?.count ?? 0) + (adding ? 1 : -1),
    byMe: true,
  });
  const set = (reactions: chat.ReactionCount[]) =>
    writeTimeline(queryClient, message.conversationId, (current) => ({
      ...current,
      messages: current.messages.map((candidate) =>
        candidate.id === message.id ? { ...candidate, reactions } : candidate,
      ),
    }));
  set(optimistic);
  try {
    const response = adding
      ? await messageClient().addReaction({ messageId: message.id, emoji })
      : await messageClient().removeReaction({ messageId: message.id, emoji });
    set(response.reactions);
  } catch (error) {
    set(before);
    throw error;
  }
}

/** Query-client bound helpers for components. */
export function useMessageActions() {
  const queryClient = useQueryClient();
  return {
    edit: async (messageId: string, text: string) => {
      const { message } = await messageClient().editMessage({ messageId, text });
      if (message) storeUpdated(queryClient, message, false);
    },
    remove: async (messageId: string) => {
      const { message } = await messageClient().deleteMessage({ messageId });
      if (message) storeUpdated(queryClient, message, false);
    },
    pin: async (message: chat.Message, pinned: boolean) => {
      const response = pinned
        ? await messageClient().pinMessage({ messageId: message.id })
        : await messageClient().unpinMessage({ messageId: message.id });
      if (response.message) storeUpdated(queryClient, response.message, false);
      void queryClient.invalidateQueries({ queryKey: chatKeys.pinned(message.conversationId) });
    },
    react: (message: chat.Message, emoji: string) => toggleReaction(queryClient, message, emoji),
  };
}
