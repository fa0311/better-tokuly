import { type QueryClient, useQueryClient } from "@tanstack/react-query";
import { type engagement, isNotFound } from "@tokuly/api";
import { useEffect } from "react";
import { realtime } from "#/lib/realtime";
import {
  commentClient,
  commentKeys,
  type SubjectKey,
  subjectTopic,
  updateReplies,
  updateThread,
  viewerStateFor,
} from "./api";
import {
  findComment,
  type ReplyPages,
  type ThreadPages,
  withComment,
  withoutComment,
  withoutReply,
  withPinned,
  withReply,
  withTotal,
  withUpdated,
  withUpdatedReply,
} from "./cache";

type Comment = engagement.Comment;

/** The comment an event is about: carried by the event, or fetched when the author's blocks kept it out. */
async function eventComment(
  carried: Comment | undefined,
  commentId: string,
): Promise<Comment | undefined> {
  if (carried) return carried;
  try {
    return (await commentClient().getComment({ commentId })).comment;
  } catch (error) {
    // Refused: this viewer is not meant to see it.
    if (isNotFound(error)) return undefined;
    throw error;
  }
}

function listed(
  queryClient: QueryClient,
  subject: SubjectKey,
  commentId: string,
): Comment | undefined {
  for (const [, data] of queryClient.getQueriesData<ThreadPages>({
    queryKey: commentKeys.subject(subject),
  })) {
    const found = findComment(data, commentId);
    if (found) return found;
  }
  return undefined;
}

function listedReply(
  queryClient: QueryClient,
  parentId: string,
  replyId: string,
): Comment | undefined {
  const data = queryClient.getQueryData<ReplyPages>(commentKeys.replies(parentId));
  return data?.pages.flatMap((page) => page.replies).find((reply) => reply.id === replyId);
}

/**
 * Keeps a subject's cached thread current from its realtime topic: comments
 * arrive, change, get pinned and leave while the thread is on screen.
 * `uuid` is the subject's UUID (topics are not addressed by short ID);
 * without it nothing is subscribed.
 */
export function useThreadRealtime(
  subject: SubjectKey,
  uuid: string | undefined,
  canModerate: boolean,
): void {
  const queryClient = useQueryClient();
  const { type, id } = subject;

  useEffect(() => {
    if (!uuid) return;
    const key: SubjectKey = { type, id };
    const client = realtime();
    const topic = subjectTopic(type, uuid);

    const apply = async (event: engagement.EngagementEvent["event"]) => {
      switch (event.case) {
        case "commentAdded": {
          const carried = await eventComment(event.value.comment, event.value.commentId);
          if (!carried) return;
          // Pushes carry no viewer state. A comment the cache already holds keeps the state it has (it may be
          // the viewer's own, just approved); a new one is someone else's.
          const known = carried.parentId
            ? listedReply(queryClient, carried.parentId, carried.id)
            : listed(queryClient, key, carried.id);
          const comment =
            known || carried.viewer ? carried : { ...carried, viewer: viewerStateFor(canModerate) };
          if (comment.parentId)
            updateReplies(queryClient, comment.parentId, (data) => withReply(data, comment));
          else updateThread(queryClient, key, (data) => withComment(data, comment, false));
          return;
        }
        case "commentUpdated": {
          const comment = await eventComment(event.value.comment, event.value.commentId);
          if (!comment) return;
          if (comment.parentId)
            updateReplies(queryClient, comment.parentId, (data) => withUpdatedReply(data, comment));
          else updateThread(queryClient, key, (data) => withUpdated(data, comment));
          return;
        }
        case "commentRemoved": {
          const { commentId, parentId } = event.value;
          // The viewer's own comment may only have been hidden or held again: they keep seeing it, with its state.
          const cached = parentId
            ? listedReply(queryClient, parentId, commentId)
            : listed(queryClient, key, commentId);
          if (cached?.viewer?.isAuthor) {
            const comment = await eventComment(undefined, commentId);
            if (comment) {
              if (comment.parentId)
                updateReplies(queryClient, comment.parentId, (data) =>
                  withUpdatedReply(data, comment),
                );
              else updateThread(queryClient, key, (data) => withUpdated(data, comment));
              return;
            }
          }
          if (parentId)
            updateReplies(queryClient, parentId, (data) => withoutReply(data, commentId));
          // Counts follow with `countersChanged`.
          else updateThread(queryClient, key, (data) => withoutComment(data, commentId, 0));
          return;
        }
        case "commentPinned": {
          const { commentId, pinned } = event.value;
          if (pinned && !listed(queryClient, key, commentId)) {
            // The pinned comment sits on a page that is not loaded: bring it in.
            const comment = await eventComment(undefined, commentId);
            if (comment)
              updateThread(queryClient, key, (data) => withComment(data, comment, false));
          }
          updateThread(queryClient, key, (data) => withPinned(data, commentId, pinned));
          return;
        }
        case "countersChanged":
          updateThread(queryClient, key, (data) => withTotal(data, event.value.commentCount));
          return;
        default:
          return;
      }
    };

    const offFrame = client.onFrame((frame) => {
      if (frame.body.case === "welcome") {
        // Whatever happened while the connection was down is caught up by reading again.
        void queryClient.invalidateQueries({ queryKey: commentKeys.subject(key) });
        return;
      }
      if (frame.body.case !== "engagement") return;
      const event = frame.body.value;
      if (event.subject?.type !== type || event.subject.id !== uuid) return;
      void apply(event.event).catch(() => {
        // A failed catch-up read only delays the update until the next refetch.
      });
    });
    const release = client.subscribe(topic);
    client.start();
    return () => {
      offFrame();
      release();
    };
  }, [queryClient, type, id, uuid, canModerate]);
}

/**
 * Keeps the owner's review queue current: `review_queue_changed` is pushed
 * to the owner's own connections whenever the number of held comments moves.
 */
export function useReviewQueueRealtime(): void {
  const queryClient = useQueryClient();
  useEffect(() => {
    const client = realtime();
    const offFrame = client.onFrame((frame) => {
      if (frame.body.case !== "engagement" || frame.body.value.event.case !== "reviewQueueChanged")
        return;
      queryClient.setQueryData(
        commentKeys.heldCount,
        Number(frame.body.value.event.value.heldCount),
      );
      void queryClient.invalidateQueries({ queryKey: commentKeys.received });
    });
    client.start();
    return offFrame;
  }, [queryClient]);
}
