import { plans } from "@tokuly/api";
import type { BadgeSource } from "#/seams/badges";
import { usePlanList, usePolls } from "./api";

/** What waits for an answer in "あそぶ": invitations nobody answered yet, plus open date polls still waiting for the person's vote. */
function usePlansBadge(): number {
  const invited = usePlanList(plans.PlanListFilter.INVITED);
  const polls = usePolls();
  const waiting =
    polls.data?.filter((poll) => poll.status === plans.PollStatus.OPEN && !poll.answeredByMe)
      .length ?? 0;
  return (invited.data?.length ?? 0) + waiting;
}

export default usePlansBadge satisfies BadgeSource;
