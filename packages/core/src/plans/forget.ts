import type { QueryClient } from "@tanstack/query-core";
import { forgetQueries } from "../query";
import { planKeys } from "./keys";

/**
 * A friendship ended: their free status goes, and what friends share (free
 * statuses, countdowns, birthdays, anniversaries, the calendar) is read again.
 */
export function forgetUserPlans(queryClient: QueryClient, _userId: string): Promise<void> {
  // `free` holds every free status, theirs too.
  return forgetQueries(queryClient, [
    planKeys.free,
    planKeys.countdowns,
    planKeys.birthdays,
    planKeys.anniversaries,
    planKeys.calendars,
  ]);
}
