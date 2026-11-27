import type { BadgeSource } from "#/seams/badges";
import { useSessionStatus } from "#/stores/session";
import { useMyCommunities } from "./api";

/**
 * What waits in the person's communities: one per community with something
 * unread, plus — for the staff — join requests and hidden posts to decide
 * about. One query that the realtime listener keeps current.
 */
function useCommunityBadge(): number {
  const signedIn = useSessionStatus() === "signedIn";
  const mine = useMyCommunities(signedIn);
  return (mine.communities ?? []).reduce(
    (sum, c) =>
      sum +
      (c.viewer?.hasUnread ? 1 : 0) +
      (c.viewer?.pendingRequestCount ?? 0) +
      (c.viewer?.reviewQueueCount ?? 0),
    0,
  );
}

export default useCommunityBadge satisfies BadgeSource;
