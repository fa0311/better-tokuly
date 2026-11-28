import type { QueryClient } from "@tanstack/query-core";
import { forgetQueries } from "../query";
import { schoolKeys } from "./keys";

/**
 * A friendship ended: their timetable and study heatmap go, and what is
 * shared among friends (timetables, who is studying, the ranking, rooms, free
 * periods) is read again.
 */
export function forgetUserSchool(queryClient: QueryClient, userId: string): Promise<void> {
  return forgetQueries(queryClient, [
    ["school", "timetable", "friend", userId],
    // `day`, `days` and `now` of another person carry their ID after the kind.
    ["school", "timetable", "day", userId],
    ["school", "timetable", "days", userId],
    schoolKeys.now(userId),
    schoolKeys.heatmap(userId),
    schoolKeys.friendTimetables,
    schoolKeys.studying,
    ["school", "study", "ranking"],
    ["school", "rooms"],
    ["school", "timetable", "free"],
  ]);
}
