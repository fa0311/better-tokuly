import type { InfiniteData, QueryClient } from "@tanstack/query-core";
import { plans } from "@tokuly/api";
import { planKeys as keys } from "./keys";

/** One page of who is free as it is cached; `mine` (the caller's own status) comes with the first page only. */
export interface FreeBoardPage {
  statuses: plans.FreeStatus[];
  mine?: plans.FreeStatus;
  nextCursor: string;
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

/** Stores a plan the server just returned, so the page and the lists show it without asking again. */
export function rememberPlan(queryClient: QueryClient, plan: plans.Plan | undefined): void {
  if (!plan) return;
  queryClient.setQueryData(keys.plan(plan.id), plan);
  void queryClient.invalidateQueries({ queryKey: keys.planLists });
  void queryClient.invalidateQueries({ queryKey: keys.calendars });
}

/** Forgets a plan the caller is no longer in (they left it), and re-reads the lists it was in. */
export function forgetPlan(queryClient: QueryClient, planId: string): void {
  queryClient.removeQueries({ queryKey: keys.plan(planId) });
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

/** Shows the caller's answer at once, before the server confirms it; returns the answer it replaced. */
export function showMyRsvp(
  queryClient: QueryClient,
  planId: string,
  rsvp: plans.Rsvp,
): plans.Rsvp | undefined {
  const before = queryClient.getQueryData<plans.Plan | null>(keys.plan(planId))?.myRsvp;
  queryClient.setQueryData<plans.Plan | null>(
    keys.plan(planId),
    (current) => current && { ...current, myRsvp: rsvp },
  );
  return before;
}

/**
 * Applies a pushed `PlansEvent` to what is loaded. Events carry IDs and
 * counters, never names or comments, so most of them only say "this changed":
 * the checklist and the free statuses are patched in place, the rest is read
 * again.
 */
export function applyPlansEvent(queryClient: QueryClient, event: plans.PlansEvent["event"]): void {
  switch (event.case) {
    case "freeStatusChanged": {
      const { userId, status } = event.value;
      patchFreeStatuses(queryClient, (statuses, pageIndex) => {
        const others = statuses.filter((existing) => existing.user?.id !== userId);
        return status && pageIndex === 0 ? [status, ...others] : others;
      });
      queryClient.setQueryData(keys.freeStatus(userId), status ?? null);
      // The server's order (running first, then by start) comes with the next read.
      void queryClient.invalidateQueries({ queryKey: keys.freeFriends });
      break;
    }
    case "freeAnswer":
      void queryClient.invalidateQueries({ queryKey: keys.free });
      break;
    case "planChanged": {
      const { planId, change } = event.value;
      if (change === plans.PlanChange.REMOVED) {
        queryClient.setQueryData(keys.plan(planId), null);
        queryClient.removeQueries({ queryKey: keys.expenses(planId) });
        queryClient.removeQueries({ queryKey: keys.split(planId) });
      } else {
        void queryClient.invalidateQueries({ queryKey: keys.plan(planId) });
      }
      void queryClient.invalidateQueries({ queryKey: keys.planLists });
      void queryClient.invalidateQueries({ queryKey: keys.calendars });
      break;
    }
    case "planRsvpChanged": {
      const { planId, counts } = event.value;
      // The counts are right at once; names, comments and the waitlist order follow.
      queryClient.setQueryData<plans.Plan | null>(keys.plan(planId), (plan) =>
        plan && counts ? { ...plan, counts } : plan,
      );
      void queryClient.invalidateQueries({ queryKey: keys.plan(planId) });
      void queryClient.invalidateQueries({ queryKey: keys.planLists });
      // Who may hold a share of an expense follows who is going.
      void queryClient.invalidateQueries({ queryKey: keys.split(planId) });
      break;
    }
    case "checklistChanged": {
      const { planId, item, removedItemId } = event.value;
      queryClient.setQueryData<plans.Plan | null>(
        keys.plan(planId),
        (plan) =>
          plan && { ...plan, checklist: mergeChecklist(plan.checklist, item, removedItemId) },
      );
      void queryClient.invalidateQueries({ queryKey: keys.planLists });
      break;
    }
    case "expensesChanged": {
      const { planId } = event.value;
      void queryClient.invalidateQueries({ queryKey: keys.expenses(planId) });
      void queryClient.invalidateQueries({ queryKey: keys.split(planId) });
      void queryClient.invalidateQueries({ queryKey: keys.plan(planId) });
      break;
    }
    case "pollChanged": {
      const changed = event.value;
      const isIt = (poll: plans.Poll | null | undefined) => poll?.id === changed.pollId;
      for (const [key, poll] of queryClient.getQueriesData<plans.Poll | null>({
        queryKey: ["plans", "poll"],
      })) {
        if (!isIt(poll)) continue;
        if (changed.deleted) queryClient.setQueryData(key, null);
        else {
          queryClient.setQueryData(key, applyTallies(poll as plans.Poll, changed));
          // Who answered what (the grid) is not in the event.
          void queryClient.invalidateQueries({ queryKey: key });
        }
      }
      void queryClient.invalidateQueries({ queryKey: keys.polls });
      void queryClient.invalidateQueries({ queryKey: keys.calendars });
      if (changed.planId) void queryClient.invalidateQueries({ queryKey: keys.planLists });
      break;
    }
    case "anniversaryChanged":
      void queryClient.invalidateQueries({ queryKey: keys.anniversaries });
      void queryClient.invalidateQueries({ queryKey: keys.calendars });
      break;
    default:
      break;
  }
}

/** The checklist with one item added, replaced or removed. */
export function mergeChecklist(
  items: plans.ChecklistItem[],
  item: plans.ChecklistItem | undefined,
  removedItemId = "",
): plans.ChecklistItem[] {
  const rest = items.filter((existing) => existing.id !== removedItemId);
  if (!item) return rest;
  return rest.some((existing) => existing.id === item.id)
    ? rest.map((existing) => (existing.id === item.id ? item : existing))
    : [...rest, item];
}

function applyTallies(poll: plans.Poll, changed: plans.PollChanged): plans.Poll {
  const tallies = new Map(changed.tallies.map((tally) => [tally.slotId, tally]));
  return {
    ...poll,
    status: changed.status || poll.status,
    answeredCount: changed.answeredCount,
    bestSlotId: changed.bestSlotId,
    planId: changed.planId || poll.planId,
    slots: poll.slots.map((slot) => {
      const tally = tallies.get(slot.id);
      return tally
        ? {
            ...slot,
            yesCount: tally.yesCount,
            maybeCount: tally.maybeCount,
            noCount: tally.noCount,
          }
        : slot;
    }),
  };
}
