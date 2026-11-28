import type { QueryClient } from "@tanstack/query-core";
import { reread } from "../query";
import { mapKeys } from "./keys";

/**
 * A friendship ended: directions to them, bumps with them and their map
 * stories go, and what friends share on the map (gatherings, trips, SOS,
 * place alerts, who looked, the story tray) is read again. Their pin is each
 * app's store.
 */
export function forgetUserMap(queryClient: QueryClient, userId: string): Promise<void> {
  queryClient.removeQueries({ queryKey: ["geo", "directions", userId] });
  queryClient.removeQueries({ queryKey: mapKeys.bumps(userId) });
  queryClient.removeQueries({ queryKey: mapKeys.storyReel(userId) });
  return reread(queryClient, [
    mapKeys.friends,
    mapKeys.gatherings,
    mapKeys.trips,
    mapKeys.sos,
    mapKeys.alerts,
    mapKeys.mapViews,
    mapKeys.storyTray,
  ]);
}
