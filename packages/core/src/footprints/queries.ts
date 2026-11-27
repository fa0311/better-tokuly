import type { geo } from "@tokuly/api";
import type { ClientSource } from "../query";
import { footprintKeys as keys } from "./keys";

type Footprints = ClientSource<typeof geo.FootprintService>;

/**
 * Query options of the footprint screens on every app. `userId` empty is the
 * caller; someone else's numbers are what they show the caller, as of the
 * last moment the caller saw them precisely (FOOTPRINTS_PRIVATE otherwise,
 * not asked twice).
 */
export const footprintQueries = {
  stats: (client: Footprints, userId = "") => ({
    queryKey: keys.stats(userId),
    queryFn: () => client().getStats({ userId }),
    ...(userId ? { retry: false } : {}),
  }),
  /** `regionId` "world" or a region's ID. */
  leaderboard: (client: Footprints, regionId: string, scope: geo.LeaderboardScope) => ({
    queryKey: keys.leaderboard(regionId, scope),
    queryFn: () =>
      client().getLeaderboard({ regionId: regionId === "world" ? "" : regionId, scope, limit: 50 }),
  }),
  /** What was uncovered since the caller last looked (not marked seen by reading it). */
  recap: (client: Footprints) => ({
    queryKey: keys.recap,
    queryFn: () => client().getRecap({ markSeen: false }),
  }),
};
