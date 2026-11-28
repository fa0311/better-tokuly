import type { QueryClient } from "@tanstack/query-core";
import { reread } from "../query";
import { schoolKeys } from "./keys";

/**
 * A friendship ended: their timetable and study heatmap go, and what is
 * shared among friends (timetables, who is studying, the ranking, rooms, free
 * periods) is read again.
 */
export function forgetUserSchool(queryClient: QueryClient, userId: string): Promise<void> {
  queryClient.removeQueries({ queryKey: ["school", "timetable", "friend", userId] });
  queryClient.removeQueries({ queryKey: schoolKeys.heatmap(userId) });
  // `day`, `days` and `now` of another person carry their ID after the kind.
  queryClient.removeQueries({
    predicate: ({ queryKey }) =>
      queryKey[0] === "school" && queryKey[1] === "timetable" && queryKey[3] === userId,
  });
  return reread(queryClient, [
    schoolKeys.friendTimetables,
    schoolKeys.studying,
    ["school", "study", "ranking"],
    ["school", "rooms"],
    ["school", "timetable", "free"],
  ]);
}
