import type { InfiniteData, QueryClient } from "@tanstack/query-core";
import type { community } from "@tokuly/api";
import type { ClientSource } from "../query";
import { communityKeys as keys } from "./keys";
import type { Paged, ThreadPage } from "./queries";

/**
 * Communities, threads and replies in the query cache: what the caller does
 * and what a pushed `CommunityEvent` changes in them.
 */

type Boards = ClientSource<typeof community.CommunityBoardService>;
type ReplyPages = InfiniteData<Paged<community.Reply>>;

/** Changes the loaded replies of a thread, in whichever order they are shown. */
export function patchReplies(
  queryClient: QueryClient,
  threadUuid: string,
  patch: (replies: community.Reply[], last: boolean, newestFirst: boolean) => community.Reply[],
): void {
  for (const [key, data] of queryClient.getQueriesData<ReplyPages>({
    queryKey: keys.repliesOf(threadUuid),
  })) {
    if (!data) continue;
    const newestFirst = key[3] === true;
    queryClient.setQueryData<ReplyPages>(key, {
      ...data,
      pages: data.pages.map((page, index) => ({
        ...page,
        items: patch(page.items, index === data.pages.length - 1, newestFirst),
      })),
    });
  }
}

/**
 * Puts a reply the caller may read into the loaded lists: replaced where it
 * is already shown, otherwise added where new replies appear (the top of a
 * newest-first list; the end of an oldest-first one once it is fully loaded).
 */
export function upsertReply(queryClient: QueryClient, reply: community.Reply): void {
  for (const [key, data] of queryClient.getQueriesData<ReplyPages>({
    queryKey: keys.repliesOf(reply.threadId),
  })) {
    if (!data) continue;
    const newestFirst = key[3] === true;
    const known = data.pages.some((page) => page.items.some((each) => each.id === reply.id));
    const lastIndex = data.pages.length - 1;
    const complete = !data.pages[lastIndex]?.nextCursor;
    queryClient.setQueryData<ReplyPages>(key, {
      ...data,
      pages: data.pages.map((page, index) => {
        if (known)
          return {
            ...page,
            items: page.items.map((each) => (each.id === reply.id ? reply : each)),
          };
        if (newestFirst && index === 0) return { ...page, items: [reply, ...page.items] };
        if (!newestFirst && index === lastIndex && complete)
          return { ...page, items: [...page.items, reply] };
        return page;
      }),
    });
  }
}

/** Applies a change to a thread wherever it is loaded (its page and every list). */
export function patchThread(
  queryClient: QueryClient,
  threadUuid: string,
  patch: (thread: community.Thread) => community.Thread,
): void {
  for (const [key, data] of queryClient.getQueriesData<community.Thread | null>({
    queryKey: keys.threadsById,
  })) {
    if (data?.id === threadUuid) queryClient.setQueryData(key, patch(data));
  }
  const inPages = <T extends { items: community.Thread[]; pinned?: community.Thread[] }>(
    data: InfiniteData<T> | undefined,
  ) =>
    data && {
      ...data,
      pages: data.pages.map((page) => ({
        ...page,
        items: page.items.map((each) => (each.id === threadUuid ? patch(each) : each)),
        ...(page.pinned
          ? { pinned: page.pinned.map((each) => (each.id === threadUuid ? patch(each) : each)) }
          : {}),
      })),
    };
  queryClient.setQueriesData<InfiniteData<ThreadPage>>(
    {
      queryKey: keys.all,
      predicate: (query) =>
        query.queryKey[3] === "threads" ||
        query.queryKey[3] === "announcements" ||
        query.queryKey[1] === "followed",
    },
    inPages,
  );
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
      void queryClient.invalidateQueries({ queryKey: keys.threadLists(communityId) });
      void queryClient.invalidateQueries({ queryKey: keys.announcements(communityId) });
      void queryClient.invalidateQueries({ queryKey: keys.channels(communityId) });
      if (event.case === "threadRemoved") refreshThread(queryClient, threadId);
      break;
    }
    case "threadUpdated": {
      const { communityId, threadId } = event.value;
      refreshThread(queryClient, threadId);
      void queryClient.invalidateQueries({ queryKey: keys.threadLists(communityId) });
      void queryClient.invalidateQueries({ queryKey: keys.announcements(communityId) });
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
      if (queryClient.getQueriesData({ queryKey: keys.repliesOf(threadId) }).length === 0) break;
      boards()
        .getReply({ replyId })
        .then(({ reply }) => reply && upsertReply(queryClient, reply))
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

/** Reads a thread again wherever it is open under an address (UUID or short ID). */
function refreshThread(queryClient: QueryClient, threadUuid: string): void {
  void queryClient.invalidateQueries({
    queryKey: keys.threadsById,
    predicate: (query) => {
      const data = query.state.data as community.Thread | null | undefined;
      return data?.id === threadUuid || query.queryKey[2] === threadUuid;
    },
  });
}
