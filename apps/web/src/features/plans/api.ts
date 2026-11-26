import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { plans, social } from "@tokuly/api";
import { type FreeBoardPage, planKeys as keys, planQueries } from "@tokuly/core/plans";
import { api } from "#/lib/api";
import { planTimeZone } from "./dates";

export const freeClient = () => api().service(plans.FreeService);
export const planClient = () => api().service(plans.PlanService);
export const pollClient = () => api().service(plans.PollService);
export const anniversaryClient = () => api().service(plans.AnniversaryService);
export const calendarClient = () => api().service(plans.CalendarService);

export {
  nameOf,
  patchFreeStatuses,
  planKeys as keys,
  rememberPlan,
  rememberPoll,
  setMyFreeStatus,
} from "@tokuly/core/plans";

/** Who is free: the friends whose status the caller may see, and the caller's own. */
export interface FreeBoard {
  statuses: plans.FreeStatus[];
  mine?: plans.FreeStatus;
}

/** How many rows a list asks for at a time; the rest follows through the load-more control. */
const PAGE_SIZE = 30;
const nextCursorOf = (page: { nextCursor: string }) => page.nextCursor || undefined;

/**
 * Who is free, as far as it is loaded: `data` is every loaded page in one
 * list, `paging` goes to `<LoadMore>` for the rest.
 */
export function useFreeBoard() {
  const query = useInfiniteQuery({
    queryKey: keys.freeFriends,
    queryFn: async ({ pageParam }): Promise<FreeBoardPage> => {
      const response = await freeClient().listFreeFriends({
        page: { limit: PAGE_SIZE, cursor: pageParam },
      });
      return {
        statuses: response.statuses,
        mine: response.mine,
        nextCursor: response.page?.nextCursor ?? "",
      };
    },
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
  });
  const pages = query.data?.pages;
  const data: FreeBoard | undefined = pages && {
    statuses: pages.flatMap((page) => page.statuses),
    mine: pages[0]?.mine,
  };
  return { data, isPending: query.isPending, paging: query };
}

/** A friend's status (the caller's own without `userId`); `null` when there is none to see. */
export function useFreeStatus(userId: string, enabled = true) {
  return useQuery({
    queryKey: keys.freeStatus(userId),
    queryFn: async () => (await freeClient().getFreeStatus({ userId })).status ?? null,
    enabled,
  });
}

export function useFreeAnswers() {
  const query = useInfiniteQuery({
    queryKey: keys.freeAnswers,
    queryFn: async ({ pageParam }) => {
      const response = await freeClient().listFreeAnswers({
        page: { limit: PAGE_SIZE, cursor: pageParam },
      });
      return { answers: response.answers, nextCursor: response.page?.nextCursor ?? "" };
    },
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
  });
  return {
    data: query.data?.pages.flatMap((page) => page.answers),
    isPending: query.isPending,
    paging: query,
  };
}

export function usePlan(planId: string) {
  return useQuery(planQueries.plan(planClient, planId));
}

/** The caller's plans of one kind, as far as they are loaded; `paging` goes to `<LoadMore>`. */
export function usePlanList(filter: plans.PlanListFilter) {
  const query = useInfiniteQuery(planQueries.list(planClient, filter));
  return {
    data: query.data?.pages.flatMap((page) => page.plans),
    isPending: query.isPending,
    paging: query,
  };
}

export function usePoll(pollId: string) {
  return useQuery({
    queryKey: keys.poll(pollId),
    queryFn: async () => (await pollClient().getPoll({ pollId })).poll ?? null,
  });
}

/** The polls the caller created or takes part in, as far as they are loaded. */
export function usePolls() {
  const query = useInfiniteQuery({
    queryKey: keys.polls,
    queryFn: async ({ pageParam }) => {
      const response = await pollClient().listPolls({
        page: { limit: PAGE_SIZE, cursor: pageParam },
      });
      return { polls: response.polls, nextCursor: response.page?.nextCursor ?? "" };
    },
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
  });
  return {
    data: query.data?.pages.flatMap((page) => page.polls),
    isPending: query.isPending,
    paging: query,
  };
}

export function useCountdowns() {
  return useQuery({
    queryKey: keys.countdowns,
    queryFn: async () =>
      (await anniversaryClient().listCountdowns({ page: { limit: 100, cursor: "" } })).countdowns,
  });
}

export function useAnniversaries() {
  return useQuery({
    queryKey: keys.anniversaries,
    queryFn: async () => (await anniversaryClient().listAnniversaries({})).anniversaries,
  });
}

export function useBirthdays() {
  return useQuery({
    queryKey: keys.birthdays,
    queryFn: async () => (await anniversaryClient().listBirthdays({})).birthdays,
  });
}

export function useCalendar(month: string) {
  return useQuery({
    queryKey: keys.calendar(month),
    queryFn: async () =>
      (await calendarClient().getCalendar({ month, timeZone: planTimeZone() })).entries,
  });
}

/** Every friend of the caller, for choosing invitees and audiences. */
export function useFriends() {
  return useQuery({
    queryKey: ["social", "friends", "all"],
    queryFn: async () => {
      const friends: social.Friend[] = [];
      let cursor = "";
      do {
        const page = await api()
          .service(social.FriendService)
          .listFriends({ page: { limit: 100, cursor } });
        friends.push(...page.friends);
        cursor = page.page?.nextCursor ?? "";
      } while (cursor);
      return friends;
    },
    staleTime: 60_000,
  });
}
