import { live } from "@tokuly/api";
import type { BadgeSource } from "#/seams/badges";
import { useFriendStreams } from "./api";

/** How many friends are on air right now. */
function useLiveBadge(): number {
  const friends = useFriendStreams();
  return friends.data?.filter((stream) => stream.status === live.StreamStatus.LIVE).length ?? 0;
}

export default useLiveBadge satisfies BadgeSource;
