import type { QueryClient } from "@tanstack/query-core";
import { forgetQueries } from "../query";
import { questKeys } from "./keys";

/**
 * A friendship ended: their quest profile, badges and the streak with them go,
 * and the streaks, the friends' leaderboards, season and badge owners are read
 * again.
 */
export function forgetUserQuest(queryClient: QueryClient, userId: string): Promise<void> {
  return forgetQueries(queryClient, [
    questKeys.profile(userId),
    questKeys.userBadges(userId),
    // The streak with them, and every list of streaks.
    questKeys.streaks,
    questKeys.leaderboards,
    questKeys.season,
    questKeys.badges,
  ]);
}
