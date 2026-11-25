import { useQuery } from "@tanstack/react-query";
import { create, social } from "@tokuly/api";
import { followKeys } from "@tokuly/core/friends";
import { api } from "#/lib/api";

export const followClient = () => api().service(social.FollowService);
export const circleClient = () => api().service(social.CircleService);

// Keys and the follow rules live in `@tokuly/core/friends`; the folder reads them from here.
export {
  circleKeys,
  FOLLOW_CARD,
  followKeys,
  isCloseFriends,
  refreshFollows,
  summaryFromProfile,
} from "@tokuly/core/friends";

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
