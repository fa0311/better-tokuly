import type { geo } from "@tokuly/api";

/** Query keys of the map: friends' places, ghost mode, gatherings, trips, safety, shares, map stories. */
export const mapKeys = {
  friends: ["social", "friends"] as const,
  ghost: ["geo", "ghost"] as const,
  gatherings: ["geo", "gatherings"] as const,
  trips: ["geo", "trips"] as const,
  sos: ["geo", "sos"] as const,
  alerts: ["geo", "alerts"] as const,
  shares: ["geo", "shares"] as const,
  directions: (userId: string, mode: geo.TravelMode) =>
    ["geo", "directions", userId, mode] as const,
  mapViews: ["social", "mapViews"] as const,
  bumps: (userId: string) => ["social", "bumps", userId] as const,
  storyTray: ["feed", "mapStories", "tray"] as const,
  storyReel: (userId: string) => ["feed", "mapStories", "reel", userId] as const,
};
