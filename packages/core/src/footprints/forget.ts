import type { QueryClient } from "@tanstack/query-core";
import { reread } from "../query";
import { footprintKeys } from "./keys";

/** A friendship ended: their footprints go, and the leaderboards are read again. */
export function forgetUserFootprints(queryClient: QueryClient, userId: string): Promise<void> {
  queryClient.removeQueries({ queryKey: footprintKeys.stats(userId) });
  queryClient.removeQueries({ queryKey: ["geo", "footprints", "cells", userId] });
  return reread(queryClient, [["geo", "footprints", "leaderboard"]]);
}
