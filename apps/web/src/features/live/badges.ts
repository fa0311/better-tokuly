import { live } from "@tokuly/api";
import type { BadgeSources } from "#/seams/badges";
import { useFriendStreams } from "./browse-screen";

/** How many friends are on air right now. */
function useLiveBadge(): number {
  const friends = useFriendStreams();
  return friends.data?.filter((stream) => stream.status === live.StreamStatus.LIVE).length ?? 0;
}

/** `live`: friends' streams that are on air. */
export default { live: useLiveBadge } satisfies BadgeSources;
