import { type QueryClient, useQuery } from "@tanstack/react-query";
import { type common, create, engagement, video } from "@tokuly/api";
import { journal, pushAfter, pushTo, pushToAll } from "@tokuly/core/query";
import { api } from "#/lib/api";
import { findComment, type ReplyPages, type ThreadPages } from "./cache";

export const commentClient = () => api().service(engagement.CommentService);
export const moderationClient = () => api().service(engagement.CommentModerationService);
export const reactionClient = () => api().service(engagement.ReactionService);
/** Channels are the identities a comment can be written as. */
const channelClient = () => api().service(video.ChannelService);

/** Query keys of comments. They start with `"engagement"`, the domain that serves them. */
export const commentKeys = {
  all: ["engagement", "comments"] as const,
  /** Every sort order of one subject's thread. */
  subject: (subject: SubjectKey) =>
    ["engagement", "comments", "thread", subject.type, subject.id] as const,
  thread: (subject: SubjectKey, sort: engagement.CommentSort) =>
    ["engagement", "comments", "thread", subject.type, subject.id, sort] as const,
  /** The replies of every comment of one subject. */
  repliesOf: (subject: SubjectKey) =>
    ["engagement", "comments", "replies", subject.type, subject.id] as const,
  replies: (subject: SubjectKey, commentId: string) =>
    ["engagement", "comments", "replies", subject.type, subject.id, commentId] as const,
  /** Every comment a link pointed at. */
  focusedAll: ["engagement", "comments", "one"] as const,
  focused: (commentId: string) => ["engagement", "comments", "one", commentId] as const,
  mine: ["engagement", "comments", "mine"] as const,
  received: ["engagement", "comments", "received"] as const,
  receivedList: (filter: ReceivedFilter) => ["engagement", "comments", "received", filter] as const,
  heldCount: ["engagement", "comments", "held-count"] as const,
  rules: ["engagement", "comments", "rules"] as const,
  blocked: ["engagement", "comments", "blocked"] as const,
  identities: ["engagement", "comments", "identities"] as const,
};

/** What a thread is about: the plain fields of a `SubjectRef`. */
export interface SubjectKey {
  type: common.SubjectType;
  /** The subject's UUID or short ID. */
  id: string;
}

export interface ReceivedFilter {
  status: engagement.CommentStatus;
  query: string;
  /** Only comments by whoever wrote this comment. */
  authorOfCommentId: string;
}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export function isUuid(value: string): boolean {
  return UUID.test(value);
}

/** The realtime topic of a subject; it is addressed by UUID only. */
export function subjectTopic(type: common.SubjectType, uuid: string): string {
  return `subject:${type}:${uuid}`;
}

/*
 * Every change to a cached thread or replies goes through the read journal
 * (`docs/realtime-consistency.md`, rule 2): their queries read with
 * `persister: journal`, and a change made while a read is on its way is
 * applied to its answer too, so an older read cannot bring back a removed
 * comment. Changes must therefore be idempotent (by comment ID).
 */

/** A top-level comment as cached in any sort order of a subject's thread. */
export function listedComment(
  queryClient: QueryClient,
  subject: SubjectKey,
  commentId: string,
): engagement.Comment | undefined {
  for (const [, data] of queryClient.getQueriesData<ThreadPages>({
    queryKey: commentKeys.subject(subject),
  })) {
    const found = findComment(data, commentId);
    if (found) return found;
  }
  return undefined;
}

/** Applies a change to every cached sort order of a subject's thread, and to their reads in flight. */
export function updateThread(
  queryClient: QueryClient,
  subject: SubjectKey,
  change: (data: ThreadPages) => ThreadPages,
): void {
  pushToAll<ThreadPages>(queryClient, commentKeys.subject(subject), change);
}

/** Applies a change to the cached replies of one comment, and to their reads in flight. */
export function updateReplies(
  queryClient: QueryClient,
  subject: SubjectKey,
  commentId: string,
  change: (data: ReplyPages) => ReplyPages,
): void {
  pushTo<ReplyPages>(queryClient, commentKeys.replies(subject, commentId), change);
}

/** A read (a comment fetched after a push) and the change its answer makes, if any. */
type ReadChange<T> = () => Promise<((data: T) => T) | undefined>;

/** `updateThread` for a change `read` decides: it lands after the pushes made while it ran (`pushAfter`). */
export async function updateThreadAfter(
  queryClient: QueryClient,
  subject: SubjectKey,
  read: ReadChange<ThreadPages>,
): Promise<void> {
  const answer = read();
  const threads = queryClient.getQueryCache().findAll({ queryKey: commentKeys.subject(subject) });
  await Promise.all([
    answer,
    ...threads.map((query) => pushAfter<ThreadPages>(queryClient, query.queryKey, () => answer)),
  ]);
}

/** `updateReplies` for a change `read` decides. */
export function updateRepliesAfter(
  queryClient: QueryClient,
  subject: SubjectKey,
  commentId: string,
  read: ReadChange<ReplyPages>,
): Promise<void> {
  return pushAfter<ReplyPages>(queryClient, commentKeys.replies(subject, commentId), read);
}

/** Forgets the comment a link pointed at once it, or the comment it answers, left public view. */
export function dropFocused(queryClient: QueryClient, commentId: string): void {
  pushToAll<engagement.GetCommentResponse | null>(queryClient, commentKeys.focusedAll, (focused) =>
    focused.comment?.id === commentId || focused.parent?.id === commentId ? null : focused,
  );
}

/**
 * The viewer state of a comment that arrived without one (realtime pushes
 * carry none): someone else's comment, which the subject's owner may moderate.
 */
export function viewerStateFor(canModerate: boolean): engagement.CommentViewerState {
  return create(engagement.CommentViewerStateSchema, {
    isAuthor: false,
    canEdit: false,
    canDelete: canModerate,
    canModerate,
  });
}

/** The channels the signed-in person can comment as. Empty for people without a channel. */
export function useCommentIdentities(enabled: boolean) {
  return useQuery({
    queryKey: commentKeys.identities,
    queryFn: async () =>
      (await channelClient().listMyChannels({})).channels.flatMap((channel) =>
        channel.summary ? [channel.summary] : [],
      ),
    staleTime: 5 * 60_000,
    enabled,
  });
}

/** How many comments wait for the signed-in person's review, across everything they own. */
export function useHeldCount(enabled = true) {
  return useQuery({
    queryKey: commentKeys.heldCount,
    queryFn: async ({ signal }) =>
      Number(
        (
          await moderationClient().listReceivedComments(
            { status: engagement.CommentStatus.HELD, page: { limit: 1 } },
            { signal },
          )
        ).heldCount,
      ),
    // `reviewQueueChanged` pushes the count.
    persister: journal,
    enabled,
  });
}
