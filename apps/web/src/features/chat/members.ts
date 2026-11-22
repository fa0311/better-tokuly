import { useInfiniteQuery } from "@tanstack/react-query";
import { chatKeys, conversationClient } from "./api";

/** The members of a conversation (owner first, then admins, then by join time), paged. */
export function useMembers(conversationId: string, enabled = true) {
  return useInfiniteQuery({
    queryKey: chatKeys.members(conversationId),
    queryFn: ({ pageParam }) =>
      conversationClient().listMembers({ conversationId, page: { cursor: pageParam, limit: 100 } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
    enabled,
  });
}
