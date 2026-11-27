import { type QueryClient, useQuery, useQueryClient } from "@tanstack/react-query";
import type { chat } from "@tokuly/api";
import {
  MessagePager,
  type Position,
  toggleReaction as sharedToggleReaction,
  storeCreated as storeCreatedIn,
  storeOwnRead,
  storeUpdated,
} from "@tokuly/core/chat";
import { chatKeys, conversationClient, messageClient } from "./api";
import { applyUnread, unreadVersion } from "./conversations";
import { useLive } from "./live";

/**
 * Messages in the web app's query cache: the hooks and requests that load and
 * change them. The cache edits are `@tokuly/core/chat`, re-exported here for
 * the folder.
 */
export { readTimeline, storeOwnRead, storeReaction, storeUpdated } from "@tokuly/core/chat";

/** Messages fetched per request. */
const PAGE = 40;

async function fetchPage(
  conversationId: string,
  position: Position,
): Promise<chat.ListMessagesResponse> {
  const response = await messageClient().listMessages({ conversationId, limit: PAGE, position });
  useLive.getState().setCursors(conversationId, response.readCursors);
  return response;
}

/** How the timelines are paged (`MessagePager` in `@tokuly/core/chat`). */
export const pager = new MessagePager(fetchPage);

/**
 * The messages of a conversation. The first load opens on `around` when
 * given (a search hit, a notification), else on the first unread message,
 * else at the newest. After that the timeline is kept current by the realtime
 * events and grown by `loadOlder` / `loadNewer`; it is never refetched whole.
 */
export function useTimeline(conversationId: string, around?: string) {
  return useQuery({
    queryKey: chatKeys.timeline(conversationId),
    queryFn: () => pager.first(conversationId, around),
    staleTime: Number.POSITIVE_INFINITY,
    gcTime: 10 * 60_000,
  });
}

export const loadOlder = (queryClient: QueryClient, conversationId: string) =>
  pager.loadOlder(queryClient, conversationId);
export const loadNewer = (queryClient: QueryClient, conversationId: string) =>
  pager.loadNewer(queryClient, conversationId);
export const catchUp = (queryClient: QueryClient, conversationId: string) =>
  pager.catchUp(queryClient, conversationId);
export const showNewest = (queryClient: QueryClient, conversationId: string) =>
  pager.showNewest(queryClient, conversationId);
export const showMessage = (queryClient: QueryClient, conversationId: string, messageId: string) =>
  pager.showMessage(queryClient, conversationId, messageId);

/**
 * Stores a message that was just created — one this client sent (from the
 * response) or one announced by an event — and fetches what was missed
 * before it. Returns false when the client does not know the conversation yet.
 */
export function storeCreated(queryClient: QueryClient, message: chat.Message): boolean {
  const stored = storeCreatedIn(queryClient, message);
  if (stored.missed) void catchUp(queryClient, message.conversationId);
  return stored.known;
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
export const toggleReaction = (queryClient: QueryClient, message: chat.Message, emoji: string) =>
  sharedToggleReaction(queryClient, messageClient, message, emoji);

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
