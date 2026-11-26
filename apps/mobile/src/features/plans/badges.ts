import { plans } from "@tokuly/api";
import type { BadgeSources } from "#/seams/badges";
import { usePlanList } from "./data";

/** `plans`: invitations to plans nobody answered yet. */
export default {
  plans: () => usePlanList(plans.PlanListFilter.INVITED).plans?.length ?? 0,
} satisfies BadgeSources;
