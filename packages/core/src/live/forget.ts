import type { QueryClient } from "@tanstack/query-core";
import { reread } from "../query";
import { liveKeys } from "./keys";

/**
 * A friendship ended. A stream names its channel, not the person behind it,
 * so the lists and the streams loaded are read again, and so is whom the
 * viewer blocked (an open chat leaves out their lines).
 */
export function forgetUserLive(queryClient: QueryClient, _userId: string): Promise<void> {
  return reread(queryClient, [liveKeys.lists, liveKeys.streams, liveKeys.blocked]);
}
