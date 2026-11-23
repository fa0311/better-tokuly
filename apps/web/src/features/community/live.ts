import type { QueryClient } from "@tanstack/react-query";
import type { community } from "@tokuly/api";
import {
  boardClient,
  keys,
  patchReplies,
  patchThread,
  refreshMembership,
  upsertReply,
} from "./api";

/**
 * Applies a pushed `CommunityEvent` to what is loaded.
 *
 * A push only ever says *that* something changed. Whatever it carries
 * besides IDs and counters is ignored: threads, replies and events are read
 * again through their RPCs, which apply the caller's permissions and blocks
 * (the topic is checked once, when it is followed, and carries no viewer
 * state).
 */
export function applyCommunityEvent(
  queryClient: QueryClient,
  event: community.CommunityEvent["event"],
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
      boardClient()
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
