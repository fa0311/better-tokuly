import {
  type QueryClient,
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { chat } from "@tokuly/api";
import {
  aheadOf,
  type ChatSync,
  capabilitiesOf,
  capabilitiesQuery,
  chatKeys,
  findInLists,
  LIST_FILTERS,
  type ListFilter,
  type ListPage,
  MessagePager,
  NO_UNREAD,
  putConversation,
  putInLists,
  readConversation,
  toggleReaction as sharedToggleReaction,
  storeAnswer,
  storeOwnRead,
  type Unread,
  UnreadTotals,
} from "@tokuly/core/chat";
import { journal } from "@tokuly/core/query";
import { api } from "#/lib/api";
import { useLive } from "./live";

export const conversationClient = () => api().service(chat.ConversationService);
export const messageClient = () => api().service(chat.MessageService);

/** Conversations per list page, messages per timeline page. */
const LIST_PAGE = 30;
const MESSAGE_PAGE = 40;

/**
 * Chat in the query cache. The rules (which list, what counts as unread,
 * how events change what is held, how timelines page) are
 * `@tokuly/core/chat`; this file is their wiring to the phone's API client
 * and live store.
 */

/** The unread totals of the signed-in account (`UnreadTotals`: answers and events never undo each other). */
export const unreadTotals = new UnreadTotals();

/** The timelines are paged through this; every page also brings the read cursors. */
export const pager = new MessagePager(async (conversationId, position, signal) => {
  const response = await messageClient().listMessages(
    { conversationId, limit: MESSAGE_PAGE, position },
    { signal },
  );
  for (const cursor of response.readCursors)
    useLive.getState().moveCursor(conversationId, cursor.userId, cursor.readSeq);
  return response;
});

/** Fetches one conversation and stores it everywhere it is held (events may have moved it on meanwhile). */
export async function fetchConversation(
  queryClient: QueryClient,
  id: string,
): Promise<chat.Conversation> {
  const response = await conversationClient().getConversation({ conversationId: id });
  for (const cursor of response.readCursors)
    useLive.getState().moveCursor(id, cursor.userId, cursor.readSeq);
  const conversation = aheadOf(response.conversation!, readConversation(queryClient, id));
  putInLists(queryClient, conversation);
  return conversation;
}

export const chatSync: ChatSync = { totals: unreadTotals, pager, fetchConversation };

export function useCapabilities() {
  return capabilitiesOf(useQuery(capabilitiesQuery(conversationClient)).data);
}

/** The unread totals: fetched once, then kept current by responses and realtime events. */
export function useUnread(): Unread {
  const queryClient = useQueryClient();
  return useQuery(unreadTotals.query(queryClient, conversationClient)).data ?? NO_UNREAD;
}

/** One of the conversation lists, paged; kept current by the realtime events. */
export function useConversationList(filter: ListFilter, { enabled = true } = {}) {
  const queryClient = useQueryClient();
  return useInfiniteQuery({
    enabled,
    queryKey: chatKeys.list(filter),
    queryFn: async ({ pageParam }): Promise<ListPage> => {
      const askedAt = unreadTotals.version();
      const response = await conversationClient().listConversations({
        filter: LIST_FILTERS[filter],
        page: { cursor: pageParam, limit: LIST_PAGE },
      });
      unreadTotals.apply(queryClient, response.unread, askedAt);
      return { conversations: response.conversations, nextCursor: response.page?.nextCursor ?? "" };
    },
    initialPageParam: "",
    getNextPageParam: (last) => last.nextCursor || undefined,
    staleTime: Number.POSITIVE_INFINITY,
  });
}

/** One conversation for the screen that has it open: a list row draws at once, the answer refreshes it. */
export function useConversation(id: string) {
  const queryClient = useQueryClient();
  return useQuery({
    queryKey: chatKeys.conversation(id),
    queryFn: () => fetchConversation(queryClient, id),
    placeholderData: () => findInLists(queryClient, id),
    staleTime: 60_000,
  });
}

/**
 * A conversation's messages: opened around `around` (a notification) or the
 * first unread one, then grown by the pager and kept current by events.
 */
export function useTimeline(conversationId: string, around?: string) {
  return useQuery({
    queryKey: chatKeys.timeline(conversationId),
    queryFn: ({ signal }) => pager.first(conversationId, around, signal),
    staleTime: Number.POSITIVE_INFINITY,
    gcTime: 10 * 60_000,
    // An edit, an unsend or a new message pushed while the first page was on its way stays.
    persister: journal,
  });
}

/** Marks the conversation read up to `messageId` (everything when empty); the badges follow the answer. */
export async function markRead(
  queryClient: QueryClient,
  conversationId: string,
  messageId = "",
): Promise<void> {
  const askedAt = unreadTotals.version();
  const response = await conversationClient().markRead({ conversationId, messageId });
  storeOwnRead(queryClient, conversationId, response.readSeq, response.unreadCount);
  unreadTotals.apply(queryClient, response.unread, askedAt);
}

/** Adds or removes the viewer's reaction, showing it at once and settling on the server's answer. */
export const toggleReaction = (queryClient: QueryClient, message: chat.Message, emoji: string) =>
  sharedToggleReaction(queryClient, messageClient, message, emoji);

export async function editMessage(
  queryClient: QueryClient,
  message: chat.Message,
  text: string,
): Promise<void> {
  await storeAnswer(queryClient, message.conversationId, () =>
    messageClient().editMessage({ messageId: message.id, text }),
  );
}

export async function deleteMessage(
  queryClient: QueryClient,
  message: chat.Message,
): Promise<void> {
  await storeAnswer(queryClient, message.conversationId, () =>
    messageClient().deleteMessage({ messageId: message.id }),
  );
}

/** Pins, mutes or archives a conversation for the viewer; the lists re-sort on the answer. */
export async function setPreferences(
  queryClient: QueryClient,
  conversationId: string,
  change: Pick<chat.UpdatePreferencesRequest, "pinned" | "muted" | "archived">,
): Promise<void> {
  const { conversation } = await conversationClient().updatePreferences({
    conversationId,
    ...change,
  });
  if (conversation) putConversation(queryClient, conversation);
  unreadTotals.refresh(queryClient);
}

/** Answers a message request (accept to reply, decline to drop it). */
export async function respondToRequest(
  queryClient: QueryClient,
  conversationId: string,
  accept: boolean,
): Promise<void> {
  const { conversation } = await conversationClient().respondToRequest({
    conversationId,
    accept,
  });
  if (conversation) putConversation(queryClient, conversation);
  unreadTotals.refresh(queryClient);
}
