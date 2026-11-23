import type { BadgeSources } from "#/seams/badges";
import { useHangouts } from "./api";

/** Invitations to hangouts the person has not answered yet. */
function useSpacesBadge(): number {
  return useHangouts().data?.invitations.length ?? 0;
}

/** `spaces`: what waits for an answer in the rooms area. */
export default { spaces: useSpacesBadge } satisfies BadgeSources;
