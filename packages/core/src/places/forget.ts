import type { QueryClient } from "@tanstack/query-core";
import { reread } from "../query";
import { placeKeys } from "./keys";

/**
 * A friendship ended: places are read again — their friends-only places, and
 * which friends visited a place, are no longer for the viewer.
 */
export function forgetUserPlaces(queryClient: QueryClient, _userId: string): Promise<void> {
  return reread(queryClient, [placeKeys.all]);
}
