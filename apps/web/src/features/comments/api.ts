import { type QueryClient, useQuery } from "@tanstack/react-query";
import { type common, create, engagement, video } from "@tokuly/api";
import { api } from "#/lib/api";
import type { ReplyPages, ThreadPages } from "./cache";

export const commentClient = () => api().service(engagement.CommentService);
export const moderationClient = () => api().service(engagement.CommentModerationService);
export const reactionClient = () => api().service(engagement.ReactionService);
/** Channels are the identities a comment can be written as. */
const channelClient = () => api().service(video.ChannelService);

/** Query keys of comments. They start with `"engagement"`, the domain that serves them. */
export const commentKeys = {
  all: ["engagement", "comments"] as const,
  /** Every sort order of one subject's thread. */
  subject: (subject: SubjectKey) => ["engagement", "comments", "thread", subject.type, subject.id] as const,
  thread: (subject: SubjectKey, sort: engagement.CommentSort) => ["engagement", "comments", "thread", subject.type, subject.id, sort] as const,
  replies: (commentId: string) => ["engagement", "comments", "replies", commentId] as const,
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

/** Applies a change to every cached sort order of a subject's thread. */
export function updateThread(queryClient: QueryClient, subject: SubjectKey, change: (data: ThreadPages) => ThreadPages): void {
  queryClient.setQueriesData<ThreadPages>({ queryKey: commentKeys.subject(subject) }, (data) => (data ? change(data) : data));
}

/** Applies a change to the cached replies of one comment, if they are loaded. */
export function updateReplies(queryClient: QueryClient, commentId: string, change: (data: ReplyPages) => ReplyPages): void {
  queryClient.setQueryData<ReplyPages>(commentKeys.replies(commentId), (data) => (data ? change(data) : data));
}

/**
 * The viewer state of a comment that arrived without one (realtime pushes
 * carry none): someone else's comment, which the subject's owner may moderate.
 */
export function viewerStateFor(canModerate: boolean): engagement.CommentViewerState {
  return create(engagement.CommentViewerStateSchema, { isAuthor: false, canEdit: false, canDelete: canModerate, canModerate });
}

/** The channels the signed-in person can comment as. Empty for people without a channel. */
export function useCommentIdentities(enabled: boolean) {
  return useQuery({
    queryKey: commentKeys.identities,
    queryFn: async () => (await channelClient().listMyChannels({})).channels.flatMap((channel) => (channel.summary ? [channel.summary] : [])),
    staleTime: 5 * 60_000,
    enabled,
  });
}

/** How many comments wait for the signed-in person's review, across everything they own. */
export function useHeldCount(enabled = true) {
  return useQuery({
    queryKey: commentKeys.heldCount,
    queryFn: async () => Number((await moderationClient().listReceivedComments({ status: engagement.CommentStatus.HELD, page: { limit: 1 } })).heldCount),
    enabled,
  });
}
