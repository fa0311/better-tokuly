import type { QueryClient } from "@tanstack/query-core";
import { forgetQueries } from "../query";
import { askKeys } from "./keys";

/**
 * A friendship ended: their box, answers and prompts go (and are read again
 * where they were loaded by handle, or are on screen), with the friends'
 * prompts and the round.
 */
export function forgetUserAsk(queryClient: QueryClient, _userId: string): Promise<void> {
  return forgetQueries(queryClient, [
    ["ask", "box"],
    askKeys.answersAll,
    askKeys.promptsAll,
    askKeys.round,
  ]);
}
