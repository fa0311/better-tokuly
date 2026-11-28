import type { QueryClient } from "@tanstack/query-core";
import { forgetQueries } from "../query";
import { footprintKeys } from "./keys";

/** A friendship ended: their footprints go, and the leaderboards are read again. */
export function forgetUserFootprints(queryClient: QueryClient, userId: string): Promise<void> {
  return forgetQueries(queryClient, [
    footprintKeys.stats(userId),
    ["geo", "footprints", "cells", userId],
    ["geo", "footprints", "leaderboard"],
  ]);
}
