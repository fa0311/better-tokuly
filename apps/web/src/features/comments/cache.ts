import type { InfiniteData } from "@tanstack/react-query";
import type { engagement } from "@tokuly/api";

/** The cached top-level comments of a subject, page by page. */
export type ThreadPages = InfiniteData<engagement.ListCommentsResponse>;
/** The cached replies of one comment, oldest first. */
export type ReplyPages = InfiniteData<engagement.ListRepliesResponse>;

type Comment = engagement.Comment;

/**
 * A comment as pushed over realtime has no viewer state: keep what the cache
 * knew about the viewer (their like, what they may do) when replacing it.
 */
export function mergeComment(cached: Comment | undefined, incoming: Comment): Comment {
  return incoming.viewer || !cached?.viewer ? incoming : { ...incoming, viewer: cached.viewer };
}

function mapThread(
  data: ThreadPages,
  map: (comments: Comment[], pageIndex: number) => Comment[],
  total?: (count: bigint) => bigint,
): ThreadPages {
  return {
    ...data,
    pages: data.pages.map((page, index) => ({
      ...page,
      comments: map(page.comments, index),
      totalCount: total ? total(page.totalCount) : page.totalCount,
    })),
  };
}

function clampCount(count: bigint): bigint {
  return count < 0n ? 0n : count;
}

export function findComment(data: ThreadPages | undefined, commentId: string): Comment | undefined {
  for (const page of data?.pages ?? []) {
    const found = page.comments.find((comment) => comment.id === commentId);
    if (found) return found;
  }
  return undefined;
}

/**
 * Adds a new top-level comment at the top of the list, under the pinned one.
 * A comment that is already listed is replaced where it is. Only comments in
 * public view move the subject's count.
 */
export function withComment(data: ThreadPages, comment: Comment, counts: boolean): ThreadPages {
  if (findComment(data, comment.id)) return withUpdated(data, comment);
  return mapThread(
    data,
    (comments, index) => {
      if (index !== 0) return comments;
      const pinned = comments.filter((item) => item.pinned);
      return [...pinned, comment, ...comments.filter((item) => !item.pinned)];
    },
    counts ? (count) => count + 1n : undefined,
  );
}

/** Replaces a listed comment (an edit, a heart, a new reply count); unknown comments are ignored. */
export function withUpdated(data: ThreadPages, comment: Comment): ThreadPages {
  if (!findComment(data, comment.id)) return data;
  return mapThread(data, (comments) =>
    comments.map((item) => (item.id === comment.id ? mergeComment(item, comment) : item)),
  );
}

/** Changes one listed comment in place. */
export function withPatched(
  data: ThreadPages,
  commentId: string,
  patch: (comment: Comment) => Comment,
): ThreadPages {
  if (!findComment(data, commentId)) return data;
  return mapThread(data, (comments) =>
    comments.map((item) => (item.id === commentId ? patch(item) : item)),
  );
}

/**
 * Drops a comment; `removedCount` (itself plus its public replies) comes off
 * the subject's count. A comment that is not listed changes nothing.
 */
export function withoutComment(
  data: ThreadPages,
  commentId: string,
  removedCount: number,
): ThreadPages {
  if (!findComment(data, commentId)) return data;
  return mapThread(
    data,
    (comments) => comments.filter((item) => item.id !== commentId),
    (count) => clampCount(count - BigInt(removedCount)),
  );
}

/**
 * One reply more (`by` 1) or less (-1) under a listed comment that had
 * `from` replies when it happened. The count becomes at least (at most)
 * `from + by`, so applying it again, or to a list read after it, changes
 * nothing.
 */
export function withReplyCountMoved(
  data: ThreadPages,
  commentId: string,
  from: number,
  by: 1 | -1,
): ThreadPages {
  const to = Math.max(0, from + by);
  return withPatched(data, commentId, (comment) => ({
    ...comment,
    replyCount: by > 0 ? Math.max(comment.replyCount, to) : Math.min(comment.replyCount, to),
  }));
}

/** Moves the pin: at most one comment is pinned, and it leads the first page. */
export function withPinned(data: ThreadPages, commentId: string, pinned: boolean): ThreadPages {
  const target = findComment(data, commentId);
  const unpinned = mapThread(data, (comments) =>
    comments.map((item) => (item.pinned ? { ...item, pinned: false } : item)),
  );
  if (!pinned || !target) return unpinned;
  return mapThread(unpinned, (comments, index) => {
    const rest = comments.filter((item) => item.id !== commentId);
    return index === 0 ? [{ ...target, pinned: true }, ...rest] : rest;
  });
}

/** Sets the subject's comment count as the server reports it. */
export function withTotal(data: ThreadPages, total: bigint): ThreadPages {
  return mapThread(
    data,
    (comments) => comments,
    () => total,
  );
}

function mapReplies(
  data: ReplyPages,
  map: (replies: Comment[], pageIndex: number, last: boolean) => Comment[],
): ReplyPages {
  return {
    ...data,
    pages: data.pages.map((page, index) => ({
      ...page,
      replies: map(page.replies, index, index === data.pages.length - 1),
    })),
  };
}

function hasReply(data: ReplyPages, replyId: string): boolean {
  return data.pages.some((page) => page.replies.some((reply) => reply.id === replyId));
}

/**
 * Appends a new reply. While older pages are still to be loaded the reply is
 * left out — it will arrive with its page — unless `force` says the viewer
 * just wrote it and has to see it.
 */
export function withReply(data: ReplyPages, reply: Comment, force = false): ReplyPages {
  if (hasReply(data, reply.id)) return withUpdatedReply(data, reply);
  const complete = !data.pages.at(-1)?.page?.nextCursor;
  if (!complete && !force) return data;
  return mapReplies(data, (replies, _index, last) => (last ? [...replies, reply] : replies));
}

export function withUpdatedReply(data: ReplyPages, reply: Comment): ReplyPages {
  if (!hasReply(data, reply.id)) return data;
  return mapReplies(data, (replies) =>
    replies.map((item) => (item.id === reply.id ? mergeComment(item, reply) : item)),
  );
}

export function withPatchedReply(
  data: ReplyPages,
  replyId: string,
  patch: (reply: Comment) => Comment,
): ReplyPages {
  if (!hasReply(data, replyId)) return data;
  return mapReplies(data, (replies) =>
    replies.map((item) => (item.id === replyId ? patch(item) : item)),
  );
}

export function withoutReply(data: ReplyPages, replyId: string): ReplyPages {
  if (!hasReply(data, replyId)) return data;
  return mapReplies(data, (replies) => replies.filter((item) => item.id !== replyId));
}
