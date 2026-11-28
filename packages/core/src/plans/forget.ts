import type { QueryClient } from "@tanstack/query-core";
import { reread } from "../query";
import { planKeys } from "./keys";

/**
 * A friendship ended: their free status goes, and what friends share (free
 * statuses, countdowns, birthdays, anniversaries, the calendar) is read again.
 */
export function forgetUserPlans(queryClient: QueryClient, userId: string): Promise<void> {
  queryClient.removeQueries({ queryKey: planKeys.freeStatus(userId) });
  return reread(queryClient, [
    planKeys.free,
    planKeys.countdowns,
    planKeys.birthdays,
    planKeys.anniversaries,
    planKeys.calendars,
  ]);
}
