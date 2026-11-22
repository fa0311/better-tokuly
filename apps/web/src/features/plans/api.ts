import {
  type InfiniteData,
  type QueryClient,
  useInfiniteQuery,
  useQuery,
} from "@tanstack/react-query";
import { type common, plans, social } from "@tokuly/api";
import { api } from "#/lib/api";
import { planTimeZone } from "./dates";

export const freeClient = () => api().service(plans.FreeService);
export const planClient = () => api().service(plans.PlanService);
export const pollClient = () => api().service(plans.PollService);
export const anniversaryClient = () => api().service(plans.AnniversaryService);
export const calendarClient = () => api().service(plans.CalendarService);

/** Query keys of this area; all start with `plans`, so one invalidation refreshes everything. */
export const keys = {
  all: ["plans"] as const,
  free: ["plans", "free"] as const,
  freeFriends: ["plans", "free", "friends"] as const,
  freeAnswers: ["plans", "free", "answers"] as const,
  freeAlerts: ["plans", "free", "alerts"] as const,
  freeStatuses: ["plans", "free", "status"] as const,
  /** One person's status; the caller's own under the empty ID. */
  freeStatus: (userId: string) => ["plans", "free", "status", userId] as const,
  planLists: ["plans", "list"] as const,
  planList: (filter: plans.PlanListFilter) => ["plans", "list", filter] as const,
  plan: (planId: string) => ["plans", "plan", planId] as const,
  expenses: (planId: string) => ["plans", "expenses", planId] as const,
  split: (planId: string) => ["plans", "split", planId] as const,
  polls: ["plans", "polls"] as const,
  /** `pollId` is whatever the address names: the ID or the short ID of the link. */
  poll: (pollId: string) => ["plans", "poll", pollId] as const,
  countdowns: ["plans", "countdowns"] as const,
  countdown: (countdownId: string) => ["plans", "countdown", countdownId] as const,
  anniversaries: ["plans", "anniversaries"] as const,
  birthdays: ["plans", "birthdays"] as const,
  settings: ["plans", "settings"] as const,
  calendar: (month: string) => ["plans", "calendar", month] as const,
  calendars: ["plans", "calendar"] as const,
};

/** Who is free: the friends whose status the caller may see, and the caller's own. */
export interface FreeBoard {
  statuses: plans.FreeStatus[];
  mine?: plans.FreeStatus;
}

/** One page of the board as it is cached; `mine` comes with the first page only. */
interface FreeBoardPage extends FreeBoard {
  nextCursor: string;
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

/** Rewrites the loaded statuses of friends, page by page (the first page has index 0). */
export function patchFreeStatuses(
  queryClient: QueryClient,
  patch: (statuses: plans.FreeStatus[], pageIndex: number) => plans.FreeStatus[],
): void {
  queryClient.setQueryData<InfiniteData<FreeBoardPage, string>>(
    keys.freeFriends,
    (board) =>
      board && {
        ...board,
        pages: board.pages.map((page, index) => ({
          ...page,
          statuses: patch(page.statuses, index),
        })),
      },
  );
}

/** Sets (or clears) the caller's own status in what is loaded. */
export function setMyFreeStatus(
  queryClient: QueryClient,
  mine: plans.FreeStatus | undefined,
): void {
  queryClient.setQueryData<InfiniteData<FreeBoardPage, string>>(
    keys.freeFriends,
    (board) =>
      board && {
        ...board,
        pages: board.pages.map((page, index) => (index === 0 ? { ...page, mine } : page)),
      },
  );
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
  return useQuery({
    queryKey: keys.plan(planId),
    queryFn: async () => (await planClient().getPlan({ planId })).plan ?? null,
  });
}

/** The caller's plans of one kind, as far as they are loaded; `paging` goes to `<LoadMore>`. */
export function usePlanList(filter: plans.PlanListFilter) {
  const query = useInfiniteQuery({
    queryKey: keys.planList(filter),
    queryFn: async ({ pageParam }) => {
      const response = await planClient().listPlans({
        filter,
        includeDeclined: false,
        page: { limit: PAGE_SIZE, cursor: pageParam },
      });
      return { plans: response.plans, nextCursor: response.page?.nextCursor ?? "" };
    },
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
  });
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

/** Stores a plan the server just returned, so the page and the lists show it without asking again. */
export function rememberPlan(queryClient: QueryClient, plan: plans.Plan | undefined): void {
  if (!plan) return;
  queryClient.setQueryData(keys.plan(plan.id), plan);
  void queryClient.invalidateQueries({ queryKey: keys.planLists });
  void queryClient.invalidateQueries({ queryKey: keys.calendars });
}

/** Stores a poll the server just returned under both of its addresses. */
export function rememberPoll(queryClient: QueryClient, poll: plans.Poll | undefined): void {
  if (!poll) return;
  queryClient.setQueryData(keys.poll(poll.id), poll);
  if (poll.shortId) queryClient.setQueryData(keys.poll(poll.shortId), poll);
  void queryClient.invalidateQueries({ queryKey: keys.polls });
  void queryClient.invalidateQueries({ queryKey: keys.calendars });
}

/** The name to show for a participant of a plan or a poll. */
export function nameOf(user: common.UserSummary | undefined): string {
  return user?.displayName || (user?.handle ? `@${user.handle}` : "?");
}
