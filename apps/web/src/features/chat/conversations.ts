import {
  type InfiniteData,
  type QueryClient,
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import type { chat } from "@tokuly/api";
import { chatKeys, conversationClient, LIST_FILTERS, type ListFilter } from "./api";
import { useLive } from "./live";
import { compareConversations, laterSeq, listOf } from "./model";

/**
 * Conversations in the query cache.
 *
 * A conversation can be held in two places: in one of the three lists
 * (`chatKeys.list`) and on its own (`chatKeys.conversation`, what an open
 * conversation screen reads). Every change goes through `putConversation` /
 * `patchConversation` / `dropConversation`, which keep both in step.
 */

export interface ListPage {
  conversations: chat.Conversation[];
  nextCursor: string;
}

type ListData = InfiniteData<ListPage, string>;

const PAGE_SIZE = 30;
const FILTERS: ListFilter[] = ["inbox", "requests", "archived"];

export const NO_UNREAD = {
  badge: 0,
  unreadConversations: 0,
  pendingRequests: 0,
  archivedUnread: 0,
};
export type Unread = typeof NO_UNREAD;

function toUnread(unread: chat.UnreadCounts | undefined): Unread {
  return unread
    ? {
        badge: unread.badge,
        unreadConversations: unread.unreadConversations,
        pendingRequests: unread.pendingRequests,
        archivedUnread: unread.archivedUnread,
      }
    : NO_UNREAD;
}

/** Stores the unread totals the server just reported (list, mark-read and count responses carry them). */
export function applyUnread(queryClient: QueryClient, unread: chat.UnreadCounts | undefined): void {
  if (!unread) return;
  queryClient.setQueryData<Unread>(chatKeys.unread, toUnread(unread));
  // A count request still under way was asked before this answer and must not replace it.
  void queryClient.cancelQueries({ queryKey: chatKeys.unread, exact: true });
}

/** Adjusts the unread totals for something that just happened (a realtime event). */
export function adjustUnread(queryClient: QueryClient, change: (unread: Unread) => Unread): void {
  queryClient.setQueryData<Unread>(chatKeys.unread, (unread) => {
    if (!unread) return unread;
    const next = change(unread);
    return {
      badge: Math.max(0, next.badge),
      unreadConversations: Math.max(0, next.unreadConversations),
      pendingRequests: Math.max(0, next.pendingRequests),
      archivedUnread: Math.max(0, next.archivedUnread),
    };
  });
}

export function refreshUnread(queryClient: QueryClient): void {
  void queryClient.invalidateQueries({ queryKey: chatKeys.unread, exact: true });
}

/** The unread totals: fetched once, then kept current by responses and realtime events. */
export function useUnread(): Unread {
  const { data } = useQuery({
    queryKey: chatKeys.unread,
    queryFn: async () => toUnread((await conversationClient().getUnreadCounts({})).unread),
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
      const response = await conversationClient().listConversations({
        filter: LIST_FILTERS[filter],
        page: { cursor: pageParam, limit: PAGE_SIZE },
      });
      applyUnread(queryClient, response.unread);
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
 * A fetched conversation, unless events moved the held copy past it while the
 * answer was on its way: then what the events brought (the newest message,
 * the unread counts) stays.
 */
function aheadOf(
  fetched: chat.Conversation,
  held: chat.Conversation | undefined,
): chat.Conversation {
  if (!held || held.lastSeq <= fetched.lastSeq) return fetched;
  const viewer =
    fetched.viewer && held.viewer
      ? {
          ...fetched.viewer,
          unreadCount: held.viewer.unreadCount,
          unreadMentionCount: held.viewer.unreadMentionCount,
          readSeq: laterSeq(held.viewer.readSeq, fetched.viewer.readSeq),
        }
      : fetched.viewer;
  return {
    ...fetched,
    lastMessage: held.lastMessage,
    lastSeq: held.lastSeq,
    lastActivityAt: held.lastActivityAt,
    viewer,
  };
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

function lists(queryClient: QueryClient): [ListFilter, ListData][] {
  return FILTERS.flatMap((filter) => {
    const data = queryClient.getQueryData<ListData>(chatKeys.list(filter));
    return data ? [[filter, data] as [ListFilter, ListData]] : [];
  });
}

function findInLists(queryClient: QueryClient, id: string): chat.Conversation | undefined {
  for (const [, data] of lists(queryClient)) {
    for (const page of data.pages) {
      const found = page.conversations.find((conversation) => conversation.id === id);
      if (found) return found;
    }
  }
  return undefined;
}

/** The copy of a conversation the client holds, if any. */
export function readConversation(
  queryClient: QueryClient,
  id: string,
): chat.Conversation | undefined {
  return (
    queryClient.getQueryData<chat.Conversation>(chatKeys.conversation(id)) ??
    findInLists(queryClient, id)
  );
}

/**
 * The rows of a list after a conversation changed: taken out where it no
 * longer belongs, put in (in order) where it does. A conversation that would
 * come after everything loaded so far is left to the pages still to load.
 */
export function listAfter(
  data: ListData,
  filter: ListFilter,
  conversation: chat.Conversation,
): ListData {
  const held = data.pages.flatMap((page) => page.conversations);
  const without = held.filter((candidate) => candidate.id !== conversation.id);
  const nextCursor = data.pages.at(-1)?.nextCursor ?? "";
  let rows = without;
  if (listOf(conversation) === filter) {
    rows = [...without, conversation].sort(compareConversations);
    const wasHeld = without.length !== held.length;
    if (nextCursor && !wasHeld && rows.at(-1) === conversation) rows = without;
  }
  if (rows.length === held.length && rows.every((row, index) => row === held[index])) return data;
  return { pages: [{ conversations: rows, nextCursor }], pageParams: [data.pageParams[0] ?? ""] };
}

function putInLists(queryClient: QueryClient, conversation: chat.Conversation): void {
  for (const [filter, data] of lists(queryClient)) {
    const next = listAfter(data, filter, conversation);
    if (next !== data) queryClient.setQueryData<ListData>(chatKeys.list(filter), next);
  }
}

/** Stores a conversation as the server sent it for this viewer (`viewer` set). */
export function putConversation(queryClient: QueryClient, conversation: chat.Conversation): void {
  queryClient.setQueryData<chat.Conversation>(chatKeys.conversation(conversation.id), (held) =>
    held ? conversation : undefined,
  );
  putInLists(queryClient, conversation);
}

/** Changes the held copy of a conversation. Returns false when the client does not hold it. */
export function patchConversation(
  queryClient: QueryClient,
  id: string,
  change: (conversation: chat.Conversation) => chat.Conversation,
): boolean {
  const held = readConversation(queryClient, id);
  if (!held) return false;
  const next = change(held);
  if (next === held) return true;
  queryClient.setQueryData<chat.Conversation>(chatKeys.conversation(id), (current) =>
    current ? next : undefined,
  );
  putInLists(queryClient, next);
  return true;
}

/** Forgets a conversation that left the viewer's lists (left, removed, hidden, declined). */
export function dropConversation(queryClient: QueryClient, id: string): void {
  for (const [filter, data] of lists(queryClient)) {
    const pages = data.pages.map((page) => ({
      ...page,
      conversations: page.conversations.filter((conversation) => conversation.id !== id),
    }));
    if (
      pages.some(
        (page, index) => page.conversations.length !== data.pages[index]!.conversations.length,
      )
    ) {
      queryClient.setQueryData<ListData>(chatKeys.list(filter), { ...data, pages });
    }
  }
  queryClient.removeQueries({ queryKey: chatKeys.conversation(id), exact: true });
  queryClient.removeQueries({ queryKey: chatKeys.timeline(id), exact: true });
}

/** Re-reads the lists from the server (after a reconnect, or when in doubt). */
export function rereadLists(queryClient: QueryClient): void {
  void queryClient.invalidateQueries({ queryKey: chatKeys.lists });
}
