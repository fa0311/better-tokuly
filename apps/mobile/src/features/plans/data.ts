import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { plans } from "@tokuly/api";
import { DEFAULT_TIME_ZONE } from "@tokuly/core/format";
import { planQueries } from "@tokuly/core/plans";
import { useMe } from "#/features/session/store";
import { api } from "#/lib/api";

/**
 * Plans in the query cache. The keys, query options, rules and cache edits
 * are `@tokuly/core/plans`; this file wires them to the phone's API client
 * (`api()` when called: whoever is signed in now) and the account's clock.
 */
export const planClient = () => api().service(plans.PlanService);

/**
 * The time zone plans are read and written in: the account's, wherever the
 * phone is (someone travelling keeps their days), as on the web.
 */
export function usePlanZone(): string {
  return useMe()?.timeZone || DEFAULT_TIME_ZONE;
}

/** One plan with its participants; `null` when the viewer is not in it (any more). */
export function usePlan(planId: string) {
  return useQuery({ ...planQueries.plan(planClient, planId), enabled: planId !== "" });
}

/** The viewer's plans of one kind, as far as they are loaded. */
export function usePlanList(filter: plans.PlanListFilter) {
  const query = useInfiniteQuery(planQueries.list(planClient, filter));
  return { plans: query.data?.pages.flatMap((page) => page.plans), paging: query };
}
