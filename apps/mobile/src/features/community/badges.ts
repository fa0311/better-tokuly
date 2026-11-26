import type { BadgeSources } from "#/seams/badges";
import { useMyCommunities } from "./data";

/**
 * What waits in the caller's communities: one per community with something
 * unread, plus — for the staff — join requests and hidden posts to decide
 * about (the staff decides on the web screens).
 */
function useCommunityBadge(): number {
  const mine = useMyCommunities();
  return (mine.communities ?? []).reduce(
    (sum, c) =>
      sum +
      (c.viewer?.hasUnread ? 1 : 0) +
      (c.viewer?.pendingRequestCount ?? 0) +
      (c.viewer?.reviewQueueCount ?? 0),
    0,
  );
}

export default { community: useCommunityBadge } satisfies BadgeSources;
