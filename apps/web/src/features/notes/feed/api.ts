import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { feed } from "@tokuly/api";
import { api } from "#/lib/api";

export const activityClient = () => api().service(feed.ActivityService);
export const homeFeedClient = () => api().service(feed.HomeFeedService);
export const recapClient = () => api().service(feed.RecapService);

/** Query keys of the feed domain (the activity log, the home feed, recaps). */
export const feedKeys = {
  all: ["feed"] as const,
  home: ["feed", "home"] as const,
  activity: ["feed", "activity"] as const,
  mine: (kindPrefix: string) => ["feed", "activity", "mine", kindPrefix] as const,
  friends: (kindPrefix: string) => ["feed", "activity", "friends", kindPrefix] as const,
  user: (userId: string) => ["feed", "activity", "user", userId] as const,
  preferences: ["feed", "preferences"] as const,
  recap: (period: feed.RecapPeriod, anchorDate: string) => ["feed", "recap", period, anchorDate] as const,
};

const PAGE_SIZE = 30;

/** The mixed home feed: friends' notes, stories, videos, bumps… as typed cards, newest first. */
export function useHomeFeed(enabled = true) {
  return useInfiniteQuery({
    queryKey: feedKeys.home,
    queryFn: ({ pageParam }) => homeFeedClient().listHomeFeed({ page: { cursor: pageParam, limit: PAGE_SIZE } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
    enabled,
    staleTime: 60_000,
  });
}

/** The viewer's own log: everything they did, private entries included. */
export function useMyActivity(kindPrefix: string) {
  return useInfiniteQuery({
    queryKey: feedKeys.mine(kindPrefix),
    queryFn: ({ pageParam }) => activityClient().listMyActivity({ kindPrefix, page: { cursor: pageParam, limit: PAGE_SIZE } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
}

/** What the viewer's friends have been doing, bursts collapsed into one card. */
export function useFriendsActivity(kindPrefix: string) {
  return useInfiniteQuery({
    queryKey: feedKeys.friends(kindPrefix),
    queryFn: ({ pageParam }) => activityClient().listFriendsActivity({ kindPrefix, page: { cursor: pageParam, limit: PAGE_SIZE } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
}

/** The activity list on a profile, as far as the viewer may see it. */
export function useUserActivity(userId: string, enabled = true) {
  return useInfiniteQuery({
    queryKey: feedKeys.user(userId),
    queryFn: ({ pageParam }) => activityClient().listUserActivity({ userId, page: { cursor: pageParam, limit: PAGE_SIZE } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
    enabled,
  });
}

/** Which kinds of the viewer's activity other people get to see. */
export function useActivityPreferences(enabled = true) {
  return useQuery({ queryKey: feedKeys.preferences, queryFn: async () => (await activityClient().getActivityPreferences({})).kinds, enabled });
}

/** The recap of the month or year containing `anchorDate` (`YYYY-MM-DD`, empty = today). */
export function useRecap(period: feed.RecapPeriod, anchorDate: string) {
  return useQuery({
    queryKey: feedKeys.recap(period, anchorDate),
    queryFn: async () => (await recapClient().getRecap({ period, anchorDate })).recap,
    staleTime: 5 * 60_000,
  });
}
