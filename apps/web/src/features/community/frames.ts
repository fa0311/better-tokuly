import type { QueryClient } from "@tanstack/react-query";
import type { community } from "@tokuly/api";
import { applyCommunityEvent } from "@tokuly/core/community";
import { boardClient } from "./api";

/** Applies a community event to what is loaded; `background.tsx` loads this with the first one. */
export function applyCommunityFrame(
  queryClient: QueryClient,
  event: community.CommunityEvent["event"],
): void {
  applyCommunityEvent(queryClient, event, boardClient);
}
