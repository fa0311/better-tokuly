import { useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { engagement } from "@tokuly/api";
import {
  type Engagement,
  engagementKeys,
  LIKE,
  reactionFields,
  type Subject,
  toEngagement,
  withReactionChange,
} from "@tokuly/core/engagement";
import { nextCursor } from "@tokuly/core/query";
import { Alert } from "react-native";
import { api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";

/**
 * Reactions and comments of anything with a subject (a short, for now). The
 * rules are `@tokuly/core/engagement`; this is their wiring to the phone.
 */
export const engagementClient = () => api().service(engagement.EngagementService);
export const reactionClient = () => api().service(engagement.ReactionService);
export const commentClient = () => api().service(engagement.CommentService);

/** Counts and the viewer's own reactions; `null` when the subject is gone or hidden. */
export function useEngagement(subject: Subject, enabled = true) {
  return useQuery({
    queryKey: engagementKeys.subject(subject),
    queryFn: async (): Promise<Engagement | null> => {
      const { engagement: found } = await engagementClient().getEngagement({ subject });
      return found ? toEngagement(found) : null;
    },
    enabled: enabled && Boolean(subject.id),
    staleTime: 60_000,
  });
}

/**
 * The like of a subject, shown at once; the server's totals replace the
 * guess when they arrive, and a refusal puts things back.
 */
export function useLike(subject: Subject): (like: boolean) => Promise<void> {
  const queryClient = useQueryClient();
  return async (like) => {
    const key = engagementKeys.subject(subject);
    // The account's client, taken before anything is awaited.
    const reactions = reactionClient();
    await queryClient.cancelQueries({ queryKey: key, exact: true });
    const before = queryClient.getQueryData<Engagement | null>(key);
    const change = like ? { add: LIKE } : { removeAll: true as const };
    if (before) queryClient.setQueryData(key, withReactionChange(before, change));
    try {
      const response = like
        ? await reactions.setReaction({ subject, reaction: { kind: LIKE.kind, emoji: "" } })
        : await reactions.removeReaction({ subject });
      queryClient.setQueryData<Engagement | null>(key, (current) =>
        current ? { ...current, ...reactionFields(response.reactions) } : current,
      );
    } catch (error) {
      queryClient.setQueryData(key, before);
      Alert.alert(errorMessage(error, { namespace: "shorts" }));
    }
  };
}

const commentsKey = (subject: Subject) =>
  ["engagement", "comments", "thread", subject.type, subject.id] as const;

/** The comments on a subject, the top ones first. */
export function useComments(subject: Subject, enabled: boolean) {
  return useInfiniteQuery({
    queryKey: commentsKey(subject),
    queryFn: ({ pageParam }) =>
      commentClient().listComments({ subject, page: { cursor: pageParam, limit: 30 } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
    enabled,
  });
}

/** Writes a comment; the thread and the counts are read again. */
export function useComment(subject: Subject): (text: string) => Promise<boolean> {
  const queryClient = useQueryClient();
  return async (text) => {
    const comments = commentClient();
    try {
      await comments.createComment({ subject, text });
      void queryClient.invalidateQueries({ queryKey: commentsKey(subject) });
      void queryClient.invalidateQueries({ queryKey: engagementKeys.subject(subject) });
      return true;
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: "shorts" }));
      return false;
    }
  };
}
