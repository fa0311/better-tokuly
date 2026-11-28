import type { InfiniteData, QueryClient, QueryKey } from "@tanstack/query-core";
import type { community } from "@tokuly/api";
import { type ClientSource, pushAfter, pushTo, pushToAll, reread } from "../query";
import { communityKeys as keys } from "./keys";
import type { Paged, ThreadPage } from "./queries";

/**
 * Communities, threads and replies in the query cache: what the caller does
 * and what a pushed `CommunityEvent` changes in them. Threads, their lists
 * and replies are read through the journal and changed through `pushTo`,
 * so a read on its way cannot bring back a removed reply or an older count
 * (`docs/realtime-consistency.md`).
 */

type Boards = ClientSource<typeof community.CommunityBoardService>;
type ReplyPages = InfiniteData<Paged<community.Reply>>;

/** The loaded reply lists of a thread, one per order (`newestFirst` is in the key). */
function replyLists(queryClient: QueryClient, threadUuid: string) {
  return queryClient
    .getQueryCache()
    .findAll({ queryKey: keys.repliesOf(threadUuid) })
    .map((query) => ({ key: query.queryKey, newestFirst: query.queryKey[3] === true }));
}

/** Changes the loaded replies of a thread, in whichever order they are shown, and every read of them in flight. */
export function patchReplies(
  queryClient: QueryClient,
  threadUuid: string,
  patch: (replies: community.Reply[], last: boolean, newestFirst: boolean) => community.Reply[],
): void {
  for (const { key, newestFirst } of replyLists(queryClient, threadUuid))
    pushTo<ReplyPages>(queryClient, key, (data) => ({
      ...data,
      pages: data.pages.map((page, index) => ({
        ...page,
        items: patch(page.items, index === data.pages.length - 1, newestFirst),
      })),
    }));
}

/**
 * A reply put into a loaded list: replaced where it is already shown,
 * otherwise added where new replies appear (the top of a newest-first list;
 * the end of an oldest-first one once it is fully loaded).
 */
function withReply(data: ReplyPages, reply: community.Reply, newestFirst: boolean): ReplyPages {
  const known = data.pages.some((page) => page.items.some((each) => each.id === reply.id));
  const lastIndex = data.pages.length - 1;
  const complete = !data.pages[lastIndex]?.nextCursor;
  return {
    ...data,
    pages: data.pages.map((page, index) => {
      if (known)
        return { ...page, items: page.items.map((each) => (each.id === reply.id ? reply : each)) };
      if (newestFirst && index === 0) return { ...page, items: [reply, ...page.items] };
      if (!newestFirst && index === lastIndex && complete)
        return { ...page, items: [...page.items, reply] };
      return page;
    }),
  };
}

/** Puts a reply the caller may read into the loaded lists (see `withReply`). */
export function upsertReply(queryClient: QueryClient, reply: community.Reply): void {
  for (const { key, newestFirst } of replyLists(queryClient, reply.threadId))
    pushTo<ReplyPages>(queryClient, key, (data) => withReply(data, reply, newestFirst));
}

/**
 * `upsertReply` for a reply `read` fetches: it lands after what was pushed
 * while it was fetched, so a removal that followed stays (`pushAfter`).
 */
async function upsertFetchedReply(
  queryClient: QueryClient,
  threadUuid: string,
  read: () => Promise<community.Reply | undefined>,
): Promise<void> {
  const answer = read();
  await Promise.all([
    answer,
    ...replyLists(queryClient, threadUuid).map(({ key, newestFirst }) =>
      pushAfter<ReplyPages>(queryClient, key, async () => {
        const reply = await answer;
        return reply && ((data) => withReply(data, reply, newestFirst));
      }),
    ),
  ]);
}

/** Applies a change to a thread wherever it is loaded (its page and every list), and to their reads in flight. */
export function patchThread(
  queryClient: QueryClient,
  threadUuid: string,
  patch: (thread: community.Thread) => community.Thread,
): void {
  // A page still on its way under a short ID is only known to be this thread once it answers.
  pushToAll<community.Thread | null>(queryClient, keys.threadsById, (thread) =>
    thread.id === threadUuid ? patch(thread) : thread,
  );
  const inPages = <T extends { items: community.Thread[]; pinned?: community.Thread[] }>(
    data: InfiniteData<T>,
  ) => ({
    ...data,
    pages: data.pages.map((page) => ({
      ...page,
      items: page.items.map((each) => (each.id === threadUuid ? patch(each) : each)),
      ...(page.pinned
        ? { pinned: page.pinned.map((each) => (each.id === threadUuid ? patch(each) : each)) }
        : {}),
    })),
  });
  const lists = queryClient.getQueryCache().findAll({
    queryKey: keys.all,
    predicate: (query) =>
      query.queryKey[3] === "threads" ||
      query.queryKey[3] === "announcements" ||
      query.queryKey[1] === "followed",
  });
  for (const list of lists) pushTo<InfiniteData<ThreadPage>>(queryClient, list.queryKey, inPages);
}

/** After the caller's own standing somewhere changed: everything that shows it. */
export function refreshMembership(queryClient: QueryClient, communityId?: string): void {
  void queryClient.invalidateQueries({ queryKey: keys.communities });
  void queryClient.invalidateQueries({ queryKey: keys.my });
  void queryClient.invalidateQueries({ queryKey: keys.myEvents });
  void queryClient.invalidateQueries({ queryKey: keys.followed });
  if (communityId) void queryClient.invalidateQueries({ queryKey: keys.of(communityId) });
}

/** Writes a community the server just returned over every copy loaded under an address. */
export function storeCommunity(queryClient: QueryClient, next: community.Community): void {
  for (const [key, data] of queryClient.getQueriesData<community.Community | null>({
    queryKey: keys.communities,
  })) {
    if (data?.id === next.id) queryClient.setQueryData(key, next);
  }
  queryClient.setQueryData(keys.community(next.handle), next);
}

/**
 * Applies a pushed `CommunityEvent` to what is loaded.
 *
 * A push only ever says *that* something changed. Whatever it carries
 * besides IDs and counters is ignored: threads, replies and events are read
 * again through their RPCs (`boards` for a reply someone is looking at),
 * which apply the caller's permissions and blocks (the topic is checked
 * once, when it is followed, and carries no viewer state).
 */
export function applyCommunityEvent(
  queryClient: QueryClient,
  event: community.CommunityEvent["event"],
  boards: Boards,
): void {
  switch (event.case) {
    case "threadCreated":
    case "threadRemoved": {
      const { communityId, threadId } = event.value;
      const removed = event.case === "threadRemoved";
      // Replacing a read on its way: one answered before a removal would bring the thread back.
      void reread(queryClient, [
        keys.threadLists(communityId),
        keys.announcements(communityId),
        keys.channels(communityId),
        ...(removed ? [keys.followed] : []),
      ]);
      if (removed) refreshThread(queryClient, threadId);
      break;
    }
    case "threadUpdated": {
      const { communityId, threadId } = event.value;
      refreshThread(queryClient, threadId);
      void reread(queryClient, [keys.threadLists(communityId), keys.announcements(communityId)]);
      break;
    }
    case "replyCreated":
    case "replyUpdated": {
      const { threadId, replyId } = event.value;
      if (event.case === "replyCreated") {
        const replyCount = event.value.replyCount;
        patchThread(queryClient, threadId, (thread) => ({ ...thread, replyCount }));
      }
      // Only someone looking at the thread needs the reply itself.
      if (replyLists(queryClient, threadId).length === 0) break;
      upsertFetchedReply(
        queryClient,
        threadId,
        async () => (await boards().getReply({ replyId })).reply,
      )
        // Not the caller's to read (a block, a lost membership): nothing to show.
        .catch(() => {});
      break;
    }
    case "replyRemoved": {
      const { threadId, replyId, replyCount } = event.value;
      patchThread(queryClient, threadId, (thread) => ({ ...thread, replyCount }));
      patchReplies(queryClient, threadId, (replies) =>
        replies.filter((reply) => reply.id !== replyId),
      );
      break;
    }
    case "threadCountsChanged": {
      const { threadId, replyCount, likeCount, commentCount } = event.value;
      patchThread(queryClient, threadId, (thread) => ({
        ...thread,
        replyCount,
        likeCount,
        commentCount,
      }));
      break;
    }
    case "channelsChanged":
      void queryClient.invalidateQueries({ queryKey: keys.channels(event.value.communityId) });
      break;
    case "calendarEventChanged": {
      const { communityId, eventId } = event.value;
      void queryClient.invalidateQueries({ queryKey: keys.event(eventId) });
      void queryClient.invalidateQueries({ queryKey: keys.attendeeLists(eventId) });
      void queryClient.invalidateQueries({ queryKey: keys.eventLists(communityId) });
      void queryClient.invalidateQueries({ queryKey: keys.myEvents });
      break;
    }
    case "communityChanged":
      void queryClient.invalidateQueries({ queryKey: keys.communities });
      if (event.value.deleted) void queryClient.invalidateQueries({ queryKey: keys.my });
      break;
    case "membershipChanged":
      refreshMembership(queryClient, event.value.communityId);
      break;
    case "staffQueueChanged": {
      const { communityId } = event.value;
      void queryClient.invalidateQueries({ queryKey: keys.communities });
      void queryClient.invalidateQueries({ queryKey: keys.my });
      void queryClient.invalidateQueries({ queryKey: keys.requests(communityId) });
      void queryClient.invalidateQueries({ queryKey: keys.queue(communityId) });
      break;
    }
    case "authorRevealed":
      // Names who looked, never the author: the log shows it.
      void queryClient.invalidateQueries({
        queryKey: [...keys.of(event.value.communityId), "log"],
      });
      break;
    default:
      break;
  }
}

/**
 * Reads everything loaded about a community again, once its topic is in
 * effect (first, or again after a gap), and a read in flight does not count
 * (rule 1): its lists, and the threads, replies and events opened by their
 * own address, which are not keyed by the community. A thread or event page
 * still on its first read is read again too: it may be this community's.
 */
export function rereadCommunity(queryClient: QueryClient, communityId: string): Promise<void> {
  const cache = queryClient.getQueryCache();
  const opened = (prefix: QueryKey, more: (id: string) => QueryKey) =>
    cache.findAll({ queryKey: prefix }).flatMap((query) => {
      const data = query.state.data as { id: string; communityId: string } | null | undefined;
      if (data === undefined) return [query.queryKey];
      return data?.communityId === communityId ? [query.queryKey, more(data.id)] : [];
    });
  return reread(queryClient, [
    keys.of(communityId),
    ...opened(keys.threadsById, keys.repliesOf),
    ...opened(keys.eventsById, keys.attendeeLists),
  ]);
}

/**
 * Reads a thread again wherever it is open under an address (UUID or short
 * ID), replacing a read on its way. A page still on its first read may be
 * this thread under its short ID: it is read again too.
 */
function refreshThread(queryClient: QueryClient, threadUuid: string): void {
  const pages = queryClient
    .getQueryCache()
    .findAll({ queryKey: keys.threadsById })
    .filter((query) => {
      const data = query.state.data as community.Thread | null | undefined;
      return data === undefined || data?.id === threadUuid || query.queryKey[2] === threadUuid;
    })
    .map((query) => query.queryKey);
  void reread(queryClient, pages);
}
