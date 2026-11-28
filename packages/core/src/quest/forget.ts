import type { QueryClient } from "@tanstack/query-core";
import { reread } from "../query";
import { questKeys } from "./keys";

/**
 * A friendship ended: their quest profile, badges and the streak with them go,
 * and the friends' leaderboards, season and badge owners are read again.
 */
export function forgetUserQuest(queryClient: QueryClient, userId: string): Promise<void> {
  queryClient.removeQueries({ queryKey: questKeys.profile(userId) });
  queryClient.removeQueries({ queryKey: questKeys.userBadges(userId) });
  queryClient.removeQueries({ queryKey: questKeys.streak(userId) });
  return reread(queryClient, [questKeys.leaderboards, questKeys.season, questKeys.badges]);
}
