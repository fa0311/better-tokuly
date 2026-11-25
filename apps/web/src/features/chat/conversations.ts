import {
  type QueryClient,
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import type { chat } from "@tokuly/api";
import {
  adjustUnread as adjustHeldUnread,
  aheadOf,
  findInLists,
  type ListPage,
  NO_UNREAD,
  putInLists,
  readConversation,
  storeUnread,
  type Unread,
  unreadOf,
} from "@tokuly/core/chat";
import { chatKeys, conversationClient, LIST_FILTERS, type ListFilter } from "./api";
import { useLive } from "./live";

/**
 * Conversations in the web app's query cache: the hooks that load them and the
 * unread totals' version counter. The cache edits themselves are
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
 * How many realtime adjustments the unread totals have had. An answer from the server
 * reflects the moment it was asked: one asked before an adjustment may predate the
 * message behind it, and replacing the adjusted totals with it would lose that message.
 */
let adjustments = 0;

/** Taken before asking the server for anything that carries unread totals; see `applyUnread`. */
export function unreadVersion(): number {
  return adjustments;
}

/**
 * Stores the unread totals the server just reported (list, mark-read and count responses
 * carry them). `askedAt` is `unreadVersion()` from before the request: when realtime
 * events adjusted the totals meanwhile, the answer may be older than what is held, so the
 * totals are read again instead.
 */
export function applyUnread(
  queryClient: QueryClient,
  unread: chat.UnreadCounts | undefined,
  askedAt: number,
): void {
  if (!unread) return;
  if (askedAt !== adjustments) {
    refreshUnread(queryClient);
    return;
  }
  storeUnread(queryClient, unread);
}

/** Adjusts the unread totals for something that just happened (a realtime event). */
export function adjustUnread(queryClient: QueryClient, change: (unread: Unread) => Unread): void {
  if (adjustHeldUnread(queryClient, change)) adjustments += 1;
}

export function refreshUnread(queryClient: QueryClient): void {
  void queryClient.invalidateQueries({ queryKey: chatKeys.unread, exact: true });
}

/** The unread totals: fetched once, then kept current by responses and realtime events. */
export function useUnread(): Unread {
  const queryClient = useQueryClient();
  const { data } = useQuery({
    queryKey: chatKeys.unread,
    queryFn: async () => {
      const askedAt = adjustments;
      const fresh = unreadOf((await conversationClient().getUnreadCounts({})).unread);
      const held = queryClient.getQueryData<Unread>(chatKeys.unread);
      if (askedAt === adjustments || !held) return fresh;
      // A message arrived while this was asked: keep what counts it and ask once more.
      setTimeout(() => refreshUnread(queryClient));
      return held;
    },
    staleTime: Number.POSITIVE_INFINITY,
  });
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

/** Re-reads the lists from the server (after a reconnect, or when in doubt). */
export function rereadLists(queryClient: QueryClient): void {
  void queryClient.invalidateQueries({ queryKey: chatKeys.lists });
}
