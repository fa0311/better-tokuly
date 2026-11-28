import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { moments } from "@tokuly/api";
import { momentQueries } from "@tokuly/core/moments";
import { api } from "#/lib/api";

// Keys, query options, rules and cache edits live in `@tokuly/core/moments`; the folder reads them from here.

export const momentClient = () => api().service(moments.MomentService);
export const reactionClient = () => api().service(moments.MomentReactionService);

/** The caller's day: the prompt, the window, their own moment, whether friends are unlocked. */
export function useToday() {
  return useQuery(momentQueries.today(momentClient));
}

export function useFriendMoments() {
  return useInfiniteQuery(momentQueries.friends(momentClient));
}

export function useMapMoments(enabled = true) {
  return useQuery({ ...momentQueries.map(momentClient), enabled });
}

export function useMoment(momentId: string) {
  return useQuery(momentQueries.moment(momentClient, momentId));
}

export function useMomentCalendar(year: number, month: number) {
  return useQuery(momentQueries.calendar(momentClient, year, month));
}

export function useMomentSettings() {
  return useQuery(momentQueries.settings(momentClient));
}

export function useReactions(momentId: string, enabled = true) {
  return useInfiniteQuery({ ...momentQueries.reactions(reactionClient, momentId), enabled });
}

export function useRealmojis() {
  return useQuery(momentQueries.realmojis(reactionClient));
}
