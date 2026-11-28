import type { QueryClient } from "@tanstack/query-core";
import { forgetQueries, reread } from "../query";
import { mapKeys } from "./keys";

/**
 * A friendship ended: directions to them, bumps with them and their map
 * stories go, and what friends share on the map (gatherings, trips, SOS,
 * place alerts, who looked, the story tray) is read again. Their pin is each
 * app's store.
 */
export async function forgetUserMap(queryClient: QueryClient, userId: string): Promise<void> {
  // `mapKeys.friends` is every friends list too (the friends forgetter keeps them, filtered).
  const friends = reread(queryClient, [mapKeys.friends]);
  await forgetQueries(queryClient, [
    ["geo", "directions", userId],
    mapKeys.bumps(userId),
    mapKeys.storyReel(userId),
    mapKeys.gatherings,
    mapKeys.trips,
    mapKeys.sos,
    mapKeys.alerts,
    mapKeys.mapViews,
    mapKeys.storyTray,
  ]);
  await friends;
}
