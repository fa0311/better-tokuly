import type { InfiniteData, QueryClient } from "@tanstack/query-core";
import type { chat } from "@tokuly/api";
import { pushAfter, pushTo } from "../query";
import { chatKeys, type ListFilter } from "./keys";
import { compareConversations, laterSeq, listOf } from "./model";

/**
 * Conversations in the query cache.
 *
 * A conversation can be held in two places: in one of the three lists
 * (`chatKeys.list`) and on its own (`chatKeys.conversation`, what an open
 * conversation screen reads). Both are read through the journal
 * (`docs/realtime-consistency.md`, rule 2), and every change goes through
 * `putConversation` / `patchConversation` / `dropConversation` (a push) or
 * `putAnswer` (a read that is not the query's own), which keep both in step:
 * a read made before a change cannot undo it. A change is applied to each
 * copy as that copy has it, and is safe to apply again: the newer of two
 * copies is the one with the higher `lastSeq` (`aheadOf`).
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

function rowIn(data: ListData, id: string): chat.Conversation | undefined {
  for (const page of data.pages) {
    const found = page.conversations.find((conversation) => conversation.id === id);
    if (found) return found;
  }
  return undefined;
}

/** A conversation as one of the loaded lists holds it (a row is enough to start drawing). */
export function findInLists(queryClient: QueryClient, id: string): chat.Conversation | undefined {
  for (const [, data] of lists(queryClient)) {
    const found = rowIn(data, id);
    if (found) return found;
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
 * A fetched conversation, unless the held copy is further along: then what
 * the events brought (the newest message, the unread counts) stays.
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

/** A list without the conversation `id`. */
function listWithout(data: ListData, id: string): ListData {
  if (!rowIn(data, id)) return data;
  return {
    ...data,
    pages: data.pages.map((page) => ({
      ...page,
      conversations: page.conversations.filter((conversation) => conversation.id !== id),
    })),
  };
}

/**
 * Pushes `change` to every copy of the conversation `id`: the one on its own
 * and the rows of the lists, each as that copy has it. A list that does not
 * hold it takes `entering` (changed) where it belongs.
 */
function pushConversation(
  queryClient: QueryClient,
  id: string,
  change: (held: chat.Conversation) => chat.Conversation,
  entering: chat.Conversation | undefined,
): void {
  pushTo<chat.Conversation>(queryClient, chatKeys.conversation(id), change);
  for (const filter of FILTERS)
    pushTo<ListData>(queryClient, chatKeys.list(filter), (data) => {
      const held = rowIn(data, id) ?? entering;
      return held ? listAfter(data, filter, change(held)) : data;
    });
}

/** Stores a conversation as the server sent it for this viewer (`viewer` set), wherever it belongs. */
export function putConversation(queryClient: QueryClient, conversation: chat.Conversation): void {
  pushConversation(
    queryClient,
    conversation.id,
    (held) => aheadOf(conversation, held),
    conversation,
  );
}

/** Changes the held copies of a conversation. Returns false when the client does not hold it. */
export function patchConversation(
  queryClient: QueryClient,
  id: string,
  change: (conversation: chat.Conversation) => chat.Conversation,
): boolean {
  const held = readConversation(queryClient, id);
  if (!held) return false;
  pushConversation(queryClient, id, change, held);
  return true;
}

/** Forgets a conversation that left the viewer's lists (left, removed, hidden, declined). */
export function dropConversation(queryClient: QueryClient, id: string): void {
  for (const filter of FILTERS)
    pushTo<ListData>(queryClient, chatKeys.list(filter), (data) => listWithout(data, id));
  queryClient.removeQueries({ queryKey: chatKeys.conversation(id), exact: true });
  queryClient.removeQueries({ queryKey: chatKeys.timeline(id), exact: true });
}

/**
 * Stores the conversation `id` that a read other than its own queries
 * brought (`GetConversation` for the lists, a mutation's answer), then what
 * was pushed while it was on its way (`pushAfter`): a conversation removed
 * meanwhile stays removed, a newer message stays the preview. Returns the
 * answer.
 */
export async function putAnswer<Answer extends { conversation?: chat.Conversation }>(
  queryClient: QueryClient,
  id: string,
  read: () => Promise<Answer>,
): Promise<Answer> {
  // One request for every copy; each `pushAfter` starts collecting pushes before it is sent.
  let answer: Promise<Answer> | undefined;
  const answered = () => (answer ??= read());
  const after =
    <T>(change: (conversation: chat.Conversation) => (held: T) => T) =>
    async () => {
      const { conversation } = await answered();
      return conversation && change(conversation);
    };
  await Promise.all([
    pushAfter<chat.Conversation>(
      queryClient,
      chatKeys.conversation(id),
      after((conversation) => (held) => aheadOf(conversation, held)),
    ),
    ...FILTERS.map((filter) =>
      pushAfter<ListData>(
        queryClient,
        chatKeys.list(filter),
        after(
          (conversation) => (data) =>
            listAfter(data, filter, aheadOf(conversation, rowIn(data, id))),
        ),
      ),
    ),
  ]);
  return answered();
}
