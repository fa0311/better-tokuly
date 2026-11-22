import { geo } from "@tokuly/api";
import { api } from "#/lib/api";

export const footprintClient = () => api().service(geo.FootprintService);

/** Query keys: everything here starts with `["geo", "footprints"]`, so one invalidation refreshes the lot. */
export const footprintKeys = {
  all: ["geo", "footprints"] as const,
  stats: (userId: string) => ["geo", "footprints", "stats", userId] as const,
  cells: (userId: string, box: readonly number[], zoom: number) => ["geo", "footprints", "cells", userId, ...box, zoom] as const,
  leaderboard: (regionId: string, scope: geo.LeaderboardScope) => ["geo", "footprints", "leaderboard", regionId, scope] as const,
  recap: ["geo", "footprints", "recap"] as const,
};

/** A region's name in the person's language. */
export const regionName = (region: geo.Region | undefined): string => region?.localName || region?.name || "";
