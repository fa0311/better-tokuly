import { type QueryClient, useQueryClient } from "@tanstack/react-query";
import { type engagement, isNotFound } from "@tokuly/api";
import { pushTo, reread } from "@tokuly/core/query";
import { useEffect } from "react";
import { realtime } from "#/lib/realtime";
import {
  commentClient,
  commentKeys,
  dropFocused,
  listedComment,
  type SubjectKey,
  subjectTopic,
  updateReplies,
  updateRepliesAfter,
  updateThread,
  updateThreadAfter,
  viewerStateFor,
} from "./api";
import {
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

/** A comment as cached: in its parent's replies, or in any sort order of the thread. */
function listed(
  queryClient: QueryClient,
  subject: SubjectKey,
  commentId: string,
  parentId: string,
): Comment | undefined {
  if (parentId)
    return queryClient
      .getQueryData<ReplyPages>(commentKeys.replies(subject, parentId))
      ?.pages.flatMap((page) => page.replies)
      .find((reply) => reply.id === commentId);
  return listedComment(queryClient, subject, commentId);
}

/** Puts what `read` answers where the comment is listed, after what was pushed meanwhile. */
function placeAfter(
  queryClient: QueryClient,
  key: SubjectKey,
  parentId: string,
  read: () => Promise<Comment | undefined>,
  inThread: (data: ThreadPages, comment: Comment) => ThreadPages,
  inReplies: (data: ReplyPages, comment: Comment) => ReplyPages,
): Promise<void> {
  return parentId
    ? updateRepliesAfter(queryClient, key, parentId, async () => {
        const comment = await read();
        return comment && ((data) => inReplies(data, comment));
      })
    : updateThreadAfter(queryClient, key, async () => {
        const comment = await read();
        return comment && ((data) => inThread(data, comment));
      });
}

/**
 * Applies one pushed event of a subject's topic to its cached thread and
 * replies. Every change goes through the read journal (`updateThread`, and
 * `updateThreadAfter` where a comment is fetched first), so neither a read on
 * its way nor a fetch that answers late brings back a removed comment.
 */
export async function applyThreadEvent(
  queryClient: QueryClient,
  key: SubjectKey,
  canModerate: boolean,
  event: engagement.EngagementEvent["event"],
): Promise<void> {
  switch (event.case) {
    case "commentAdded": {
      const { commentId, parentId } = event.value;
      const carried = event.value.comment;
      return placeAfter(
        queryClient,
        key,
        parentId,
        async () => {
          const comment = await eventComment(carried, commentId);
          if (!comment) return undefined;
          // Pushes carry no viewer state. A comment the cache already holds keeps the state it has
          // (it may be the viewer's own, just approved); a new one is someone else's.
          const known = listed(queryClient, key, commentId, parentId);
          return known || comment.viewer
            ? comment
            : { ...comment, viewer: viewerStateFor(canModerate) };
        },
        (data, comment) => withComment(data, comment, false),
        (data, comment) => withReply(data, comment),
      );
    }
    case "commentUpdated": {
      const { commentId, parentId } = event.value;
      const carried = event.value.comment;
      return placeAfter(
        queryClient,
        key,
        parentId,
        () => eventComment(carried, commentId),
        withUpdated,
        withUpdatedReply,
      );
    }
    case "commentRemoved": {
      const { commentId, parentId } = event.value;
      const remove = () => {
        if (parentId)
          updateReplies(queryClient, key, parentId, (data) => withoutReply(data, commentId));
        // Counts follow with `countersChanged`.
        else updateThread(queryClient, key, (data) => withoutComment(data, commentId, 0));
        dropFocused(queryClient, commentId);
      };
      if (!listed(queryClient, key, commentId, parentId)?.viewer?.isAuthor) return remove();
      // The viewer's own comment may only have been hidden or held again: they keep seeing it,
      // with its state.
      let kept = false;
      await placeAfter(
        queryClient,
        key,
        parentId,
        async () => {
          const comment = await eventComment(undefined, commentId);
          kept = comment !== undefined;
          return comment;
        },
        withUpdated,
        withUpdatedReply,
      );
      if (!kept) remove();
      return;
    }
    case "commentPinned": {
      const { commentId, pinned } = event.value;
      updateThread(queryClient, key, (data) => withPinned(data, commentId, pinned));
      if (!pinned || listed(queryClient, key, commentId, "")) return;
      // The pinned comment sits on a page that is not loaded: bring it in, pinned.
      return updateThreadAfter(queryClient, key, async () => {
        const comment = await eventComment(undefined, commentId);
        return (
          comment && ((data) => withPinned(withComment(data, comment, false), commentId, true))
        );
      });
    }
    case "countersChanged": {
      const total = event.value.commentCount;
      updateThread(queryClient, key, (data) => withTotal(data, total));
      return;
    }
    default:
      return;
  }
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

    const offFrame = client.onFrame((frame) => {
      if (frame.body.case !== "engagement") return;
      const event = frame.body.value;
      if (event.subject?.type !== type || event.subject.id !== uuid) return;
      void applyThreadEvent(queryClient, key, canModerate, event.event).catch(() => {
        // A failed catch-up read only delays the update until the next read.
      });
    });
    const { release } = client.subscribe(topic, {
      // In effect (first, or back after a gap): whatever happened before, including while the
      // comments were first being read, is caught up by reading again, replies too.
      onSubscribed: () =>
        void reread(queryClient, [commentKeys.subject(key), commentKeys.repliesOf(key)]),
    });
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
      const held = Number(frame.body.value.event.value.heldCount);
      pushTo<number>(queryClient, commentKeys.heldCount, () => held);
      void queryClient.invalidateQueries({ queryKey: commentKeys.received });
    });
    client.start();
    return offFrame;
  }, [queryClient]);
}
