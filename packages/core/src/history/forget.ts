import type { QueryClient } from "@tanstack/query-core";
import { forgetQueries } from "../query";
import { historyKeys } from "./keys";

/** A friendship ended: the history is read again, without them among the friends met. */
export function forgetUserHistory(queryClient: QueryClient, _userId: string): Promise<void> {
  return forgetQueries(queryClient, [historyKeys.all]);
}
