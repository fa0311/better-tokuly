import type { QueryClient } from "@tanstack/query-core";
import { create, type identity, social } from "@tokuly/api";
import { profileKeys } from "../profile/keys";
import { followKeys } from "./keys";

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

/** After a follow, an unfollow or an answer: every count and list re-reads, profiles too. */
export function refreshFollows(queryClient: QueryClient) {
  return Promise.all([
    queryClient.invalidateQueries({ queryKey: followKeys.all }),
    queryClient.invalidateQueries({ queryKey: profileKeys.profiles }),
  ]);
}

export const isCloseFriends = (circle: social.Circle) =>
  circle.kind === social.CircleKind.CLOSE_FRIENDS;
