import { type QueryClient, useQuery } from "@tanstack/react-query";
import { create, type identity, social } from "@tokuly/api";
import { api } from "#/lib/api";

export const followClient = () => api().service(social.FollowService);
export const circleClient = () => api().service(social.CircleService);

/**
 * Query keys of follows and circles. Everything starts with
 * `["social", "follow"]` or `["social", "circles"]`, so one invalidation
 * refreshes a whole side after a change or a realtime push.
 */
export const followKeys = {
  all: ["social", "follow"] as const,
  summary: (userId: string) => ["social", "follow", "summary", userId] as const,
  lists: ["social", "follow", "list"] as const,
  userLists: (userId: string) => ["social", "follow", "list", userId] as const,
  list: (userId: string, list: social.FollowList, query: string) =>
    ["social", "follow", "list", userId, list, query] as const,
  requests: ["social", "follow", "requests"] as const,
  requestList: (direction: social.FollowRequestDirection) =>
    ["social", "follow", "requests", direction] as const,
  incomingCount: ["social", "follow", "requests", "incoming-count"] as const,
  settings: ["social", "follow", "settings"] as const,
  // Not under `all`: following someone from the list keeps them in it (as フォロー中).
  suggestions: ["social", "follow-suggestions"] as const,
};

export const circleKeys = {
  all: ["social", "circles"] as const,
  list: ["social", "circles", "list"] as const,
  circle: (id: string) => ["social", "circles", "one", id] as const,
  members: (id: string) => ["social", "circles", "members", id] as const,
  candidates: (id: string, query: string) =>
    ["social", "circles", "candidates", id, query] as const,
};

/** The profile card the social domain attaches: `social.follow` (counts and the viewer's relation). */
export const FOLLOW_CARD = "social.follow";

const STATES: Record<string, social.FollowState> = {
  NONE: social.FollowState.NONE,
  FOLLOWING: social.FollowState.FOLLOWING,
  REQUESTED: social.FollowState.REQUESTED,
};

/** What a profile's `social.follow` card says, as a summary (until the real one has loaded). */
export function summaryFromProfile(profile: identity.Profile): social.FollowSummary | undefined {
  const card = profile.cards.find((candidate) => candidate.kind === FOLLOW_CARD);
  if (!card || !profile.user) return undefined;
  const data = card.data;
  return create(social.FollowSummarySchema, {
    userId: profile.user.id,
    followerCount: Number(data.follower_count) || 0,
    followingCount: Number(data.following_count) || 0,
    locked: data.locked === "true",
    knownFollowerCount: Number(data.known_follower_count) || 0,
    relation: {
      following: STATES[data.following ?? ""] ?? social.FollowState.NONE,
      followsYou: data.follows_you === "true",
    },
  });
}

/** Someone's follow numbers and the viewer's relation to them (with their prefs when followed). */
export function useFollowSummary(userId: string | undefined, placeholder?: social.FollowSummary) {
  return useQuery({
    queryKey: followKeys.summary(userId ?? ""),
    queryFn: async () =>
      (await followClient().getFollowSummary({ userId: userId! })).summary ??
      create(social.FollowSummarySchema, { userId }),
    enabled: !!userId,
    placeholderData: placeholder,
  });
}

/** After a follow, an unfollow or an answer: every count and list re-reads, profiles too. */
export function refreshFollows(queryClient: QueryClient) {
  return Promise.all([
    queryClient.invalidateQueries({ queryKey: followKeys.all }),
    queryClient.invalidateQueries({ queryKey: ["identity", "profile"] }),
  ]);
}

/** CircleColor → the palette's background class. */
export const CIRCLE_COLORS: { color: social.CircleColor; key: string; bg: string }[] = [
  { color: social.CircleColor.BRAND, key: "brand", bg: "bg-brand" },
  { color: social.CircleColor.LIME, key: "lime", bg: "bg-lime" },
  { color: social.CircleColor.SUN, key: "sun", bg: "bg-sun" },
  { color: social.CircleColor.SKY, key: "sky", bg: "bg-sky" },
  { color: social.CircleColor.PINK, key: "pink", bg: "bg-pink" },
  { color: social.CircleColor.MINT, key: "mint", bg: "bg-mint" },
  { color: social.CircleColor.CORAL, key: "coral", bg: "bg-coral" },
];

export function circleBg(color: social.CircleColor): string {
  return CIRCLE_COLORS.find((entry) => entry.color === color)?.bg ?? "bg-brand";
}

export const isCloseFriends = (circle: social.Circle) =>
  circle.kind === social.CircleKind.CLOSE_FRIENDS;

/** Follow requests waiting for the viewer's answer (one cheap read, kept current by realtime). */
export function useFollowRequestCount(): number {
  const count = useQuery({
    queryKey: followKeys.incomingCount,
    queryFn: () =>
      followClient().listFollowRequests({
        direction: social.FollowRequestDirection.INCOMING,
        page: { limit: 1 },
      }),
  });
  return count.data?.incomingCount ?? 0;
}
