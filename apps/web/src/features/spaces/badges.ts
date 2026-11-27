import type { BadgeSource } from "#/seams/badges";
import { useHangouts } from "./api";

/** What waits for an answer in the rooms area: invitations to hangouts not answered yet. */
function useSpacesBadge(): number {
  return useHangouts().data?.invitations.length ?? 0;
}

export default useSpacesBadge satisfies BadgeSource;
