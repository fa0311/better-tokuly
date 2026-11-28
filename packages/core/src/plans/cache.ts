import type { InfiniteData, QueryClient, QueryKey } from "@tanstack/query-core";
import { plans } from "@tokuly/api";
import { pushTo, reread } from "../query";
import { planKeys as keys } from "./keys";

/** One page of who is free as it is cached; `mine` (the caller's own status) comes with the first page only. */
export interface FreeBoardPage {
  statuses: plans.FreeStatus[];
  mine?: plans.FreeStatus;
  nextCursor: string;
}

/**
 * Rewrites the loaded statuses of friends, page by page (the first page has
 * index 0), and those of a read in flight when it lands (`pushTo`).
 */
export function patchFreeStatuses(
  queryClient: QueryClient,
  patch: (statuses: plans.FreeStatus[], pageIndex: number) => plans.FreeStatus[],
): void {
  pushTo<InfiniteData<FreeBoardPage, string>>(queryClient, keys.freeFriends, (board) => ({
    ...board,
    pages: board.pages.map((page, index) => ({ ...page, statuses: patch(page.statuses, index) })),
  }));
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
 * the checklist, the counts and the free statuses are patched in place, the
 * rest is read again. Both go by `docs/realtime-consistency.md`, rule 2:
 * patches and removals through `pushTo`, so that a read made before them
 * cannot undo them, and reading again through `reread`, which replaces a read
 * in flight instead of joining it.
 */
export function applyPlansEvent(queryClient: QueryClient, event: plans.PlansEvent["event"]): void {
  const readAgain = (...queryKeys: QueryKey[]) => void reread(queryClient, queryKeys);
  switch (event.case) {
    case "freeStatusChanged": {
      const { userId, status } = event.value;
      patchFreeStatuses(queryClient, (statuses, pageIndex) => {
        const others = statuses.filter((existing) => existing.user?.id !== userId);
        return status && pageIndex === 0 ? [status, ...others] : others;
      });
      // The server's order (running first, then by start) comes with the next read, and so does a
      // status where none was held.
      readAgain(keys.freeFriends, keys.freeStatus(userId));
      break;
    }
    case "freeAnswer":
      readAgain(keys.free);
      break;
    case "planChanged": {
      const { planId, change } = event.value;
      if (change === plans.PlanChange.REMOVED) {
        pushTo<plans.Plan | null>(queryClient, keys.plan(planId), () => null);
        queryClient.removeQueries({ queryKey: keys.expenses(planId) });
        queryClient.removeQueries({ queryKey: keys.split(planId) });
        readAgain(keys.planLists, keys.calendars);
      } else readAgain(keys.plan(planId), keys.planLists, keys.calendars);
      break;
    }
    case "planRsvpChanged": {
      const { planId, counts } = event.value;
      // The counts are right at once; names, comments and the waitlist order follow.
      if (counts)
        pushTo<plans.Plan | null>(queryClient, keys.plan(planId), (plan) => ({ ...plan, counts }));
      // Who may hold a share of an expense follows who is going.
      readAgain(keys.plan(planId), keys.planLists, keys.split(planId));
      break;
    }
    case "checklistChanged": {
      const { planId, item, removedItemId } = event.value;
      pushTo<plans.Plan | null>(queryClient, keys.plan(planId), (plan) => ({
        ...plan,
        checklist: mergeChecklist(plan.checklist, item, removedItemId),
      }));
      readAgain(keys.planLists);
      break;
    }
    case "expensesChanged": {
      const { planId } = event.value;
      readAgain(keys.expenses(planId), keys.split(planId), keys.plan(planId));
      break;
    }
    case "pollChanged": {
      const changed = event.value;
      const isIt = (poll: plans.Poll) => poll.id === changed.pollId;
      // A poll is held under its ID and under the short ID of its link, and a first read does not
      // say which poll it is until it lands: the change goes to every read in flight too, and
      // looks at the poll there.
      for (const query of queryClient.getQueryCache().findAll({ queryKey: keys.everyPoll })) {
        const held = query.state.data as plans.Poll | null | undefined;
        if (held === null || (held && !isIt(held))) continue;
        pushTo<plans.Poll | null>(queryClient, query.queryKey, (poll) =>
          !isIt(poll) ? poll : changed.deleted ? null : applyTallies(poll, changed),
        );
        // Who answered what (the grid) is not in the event.
        if (held && !changed.deleted) readAgain(query.queryKey);
      }
      readAgain(keys.polls, keys.calendars, ...(changed.planId ? [keys.planLists] : []));
      break;
    }
    case "anniversaryChanged":
      readAgain(keys.anniversaries, keys.calendars);
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
