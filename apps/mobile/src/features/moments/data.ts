import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { moments } from "@tokuly/api";
import { momentKeys, momentQueries } from "@tokuly/core/moments";
import { api } from "#/lib/api";

export const momentClient = () => api().service(moments.MomentService);
const reactionClient = () => api().service(moments.MomentReactionService);

export { momentKeys };

export function useMomentToday() {
  return useQuery(momentQueries.today(momentClient));
}

export function useFriendMoments() {
  return useInfiniteQuery(momentQueries.friends(momentClient));
}

export function useMomentRealmojis() {
  return useQuery(momentQueries.realmojis(reactionClient));
}

export function useMoment(momentId: string) {
  return useQuery({
    ...momentQueries.moment(momentClient, momentId),
    enabled: Boolean(momentId),
  });
}

export function useMomentReactions(momentId: string, enabled = true) {
  return useInfiniteQuery({
    ...momentQueries.reactions(reactionClient, momentId),
    enabled: enabled && Boolean(momentId),
    // Recheck visibility and the daily unlock whenever the sheet is reopened.
    staleTime: 0,
  });
}
