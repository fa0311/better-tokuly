import type { QueryClient } from "@tanstack/query-core";
import { reread } from "../query";
import { askKeys } from "./keys";

/**
 * A friendship ended: their box, answers and prompts go (and are read again
 * where they were loaded by handle), with the friends' prompts and the round.
 */
export function forgetUserAsk(queryClient: QueryClient, userId: string): Promise<void> {
  queryClient.removeQueries({ queryKey: askKeys.box(userId) });
  queryClient.removeQueries({ queryKey: askKeys.answers(userId) });
  queryClient.removeQueries({ queryKey: askKeys.promptsOf(userId) });
  return reread(queryClient, [
    ["ask", "box"],
    askKeys.answersAll,
    askKeys.promptsAll,
    askKeys.round,
  ]);
}
