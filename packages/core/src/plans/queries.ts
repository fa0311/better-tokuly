import type { plans } from "@tokuly/api";
import { type ClientSource, journal } from "../query";
import { planKeys as keys } from "./keys";

type Plans = ClientSource<typeof plans.PlanService>;

/** How many plans a list asks for at a time. */
const PAGE_SIZE = 30;

/** One page of a list of plans, as it is cached. */
export interface PlanPage {
  plans: plans.Plan[];
  nextCursor: string;
}

/** Query options of plans. Each app adds its hooks around them. */
export const planQueries = {
  /**
   * One plan with its participants and checklist; `null` when the caller is
   * not in it. Pushes reach it through `pushTo` (see `applyPlansEvent`).
   */
  plan: (client: Plans, planId: string) => ({
    queryKey: keys.plan(planId),
    queryFn: async ({ signal }: { signal: AbortSignal }) =>
      (await client().getPlan({ planId }, { signal })).plan ?? null,
    persister: journal,
  }),

  /** The caller's plans of one kind (declined ones left out), in pages. */
  list: (client: Plans, filter: plans.PlanListFilter) => ({
    queryKey: keys.planList(filter),
    queryFn: async ({ pageParam }: { pageParam: string }): Promise<PlanPage> => {
      const response = await client().listPlans({
        filter,
        includeDeclined: false,
        page: { limit: PAGE_SIZE, cursor: pageParam },
      });
      return { plans: response.plans, nextCursor: response.page?.nextCursor ?? "" };
    },
    initialPageParam: "",
    getNextPageParam: (page: PlanPage) => page.nextCursor || undefined,
  }),
};
