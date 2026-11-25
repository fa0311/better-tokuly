import {
  type QueryClient,
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import type { chat } from "@tokuly/api";
import {
  aheadOf,
  findInLists,
  type ListPage,
  NO_UNREAD,
  putInLists,
  readConversation,
  type Unread,
  UnreadTotals,
} from "@tokuly/core/chat";
import { chatKeys, conversationClient, LIST_FILTERS, type ListFilter } from "./api";
import { useLive } from "./live";

/**
 * Conversations in the web app's query cache: the hooks that load them and the
 * unread totals. The cache edits themselves are
 * `@tokuly/core/chat` (`putConversation`, `patchConversation`, …), re-exported
 * here for the folder.
 */
export {
  dropConversation,
  type ListPage,
  listAfter,
  NO_UNREAD,
  patchConversation,
  putConversation,
  readConversation,
  type Unread,
} from "@tokuly/core/chat";

const PAGE_SIZE = 30;

/**
 * The unread totals: answers from the server and realtime adjustments,
 * without one undoing the other (`UnreadTotals` in `@tokuly/core/chat`).
 */
export const unreadTotals = new UnreadTotals();

/** Taken before asking the server for anything that carries unread totals; see `applyUnread`. */
export function unreadVersion(): number {
  return unreadTotals.version();
}

/** Stores the unread totals the server just reported; `askedAt` is `unreadVersion()` from before the request. */
export function applyUnread(
  queryClient: QueryClient,
  unread: chat.UnreadCounts | undefined,
  askedAt: number,
): void {
  unreadTotals.apply(queryClient, unread, askedAt);
}

export function refreshUnread(queryClient: QueryClient): void {
  unreadTotals.refresh(queryClient);
}

/** The unread totals: fetched once, then kept current by responses and realtime events. */
export function useUnread(): Unread {
  const queryClient = useQueryClient();
  const { data } = useQuery(unreadTotals.query(queryClient, conversationClient));
  return data ?? NO_UNREAD;
}

/** One of the conversation lists, paged; kept current by the realtime events. */
export function useConversationList(filter: ListFilter) {
  const queryClient = useQueryClient();
  return useInfiniteQuery({
    queryKey: chatKeys.list(filter),
    queryFn: async ({ pageParam }): Promise<ListPage> => {
      const askedAt = unreadVersion();
      const response = await conversationClient().listConversations({
        filter: LIST_FILTERS[filter],
        page: { cursor: pageParam, limit: PAGE_SIZE },
      });
      applyUnread(queryClient, response.unread, askedAt);
      return { conversations: response.conversations, nextCursor: response.page?.nextCursor ?? "" };
    },
    initialPageParam: "",
    getNextPageParam: (last) => last.nextCursor || undefined,
    staleTime: Number.POSITIVE_INFINITY,
  });
}

/** What opening a conversation tells us beyond the conversation itself goes to the live store. */
function noteDetails(response: chat.GetConversationResponse, askedAt: number): void {
  const conversation = response.conversation;
  if (!conversation) return;
  const live = useLive.getState();
  live.setCursors(conversation.id, response.readCursors);
  // The answer describes the moment it was asked for: events that arrived since then are newer.
  live.setPresent(conversation.id, response.presentUserIds, askedAt);
  live.setOpenPov(conversation.id, response.openPovRequest, askedAt);
  if (response.activeCall) live.setCall(response.activeCall);
}

/** Fetches one conversation with its extras and stores it everywhere it is held. */
export async function fetchConversation(
  queryClient: QueryClient,
  id: string,
): Promise<chat.Conversation> {
  const askedAt = Date.now();
  const response = await conversationClient().getConversation({ conversationId: id });
  noteDetails(response, askedAt);
  const conversation = aheadOf(response.conversation!, readConversation(queryClient, id));
  putInLists(queryClient, conversation);
  return conversation;
}

/**
 * One conversation, for the screen that has it open: served from the cache
 * (a list row is enough to start drawing) and refreshed with its extras.
 */
export function useConversation(id: string) {
  const queryClient = useQueryClient();
  return useQuery({
    queryKey: chatKeys.conversation(id),
    queryFn: () => fetchConversation(queryClient, id),
    // A row from the list is enough to start drawing; the answer adds read cursors, presence, POV and call.
    placeholderData: () => findInLists(queryClient, id),
    staleTime: 60_000,
  });
}
