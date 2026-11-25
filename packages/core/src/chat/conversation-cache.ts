import type { InfiniteData, QueryClient } from "@tanstack/query-core";
import type { chat } from "@tokuly/api";
import { chatKeys, type ListFilter } from "./keys";
import { compareConversations, laterSeq, listOf } from "./model";

/**
 * Conversations in the query cache.
 *
 * A conversation can be held in two places: in one of the three lists
 * (`chatKeys.list`) and on its own (`chatKeys.conversation`, what an open
 * conversation screen reads). Every change goes through `putConversation` /
 * `patchConversation` / `dropConversation`, which keep both in step.
 */

/** A page of a conversation list as it sits in the cache. */
export interface ListPage {
  conversations: chat.Conversation[];
  nextCursor: string;
}

export type ListData = InfiniteData<ListPage, string>;

const FILTERS: ListFilter[] = ["inbox", "requests", "archived"];

function lists(queryClient: QueryClient): [ListFilter, ListData][] {
  return FILTERS.flatMap((filter) => {
    const data = queryClient.getQueryData<ListData>(chatKeys.list(filter));
    return data ? [[filter, data] as [ListFilter, ListData]] : [];
  });
}

/** A conversation as one of the loaded lists holds it (a row is enough to start drawing). */
export function findInLists(queryClient: QueryClient, id: string): chat.Conversation | undefined {
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
 * A fetched conversation, unless events moved the held copy past it while the
 * answer was on its way: then what the events brought (the newest message,
 * the unread counts) stays.
 */
export function aheadOf(
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

/** Puts a conversation where it belongs in every loaded list. */
export function putInLists(queryClient: QueryClient, conversation: chat.Conversation): void {
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
