import type { stamp } from "@tokuly/api";

/** Query keys of the stamp domain. Everything starts with `"stamp"`, so one invalidation reaches all of it. */
export const stampKeys = {
  all: ["stamp"] as const,
  store: ["stamp", "store"] as const,
  storeHome: ["stamp", "store", "home"] as const,
  storeList: (section: stamp.StoreSection, tag: string, price: stamp.PriceFilter) =>
    ["stamp", "store", "list", section, tag, price] as const,
  storeSearch: (query: string, price: stamp.PriceFilter) =>
    ["stamp", "store", "search", query, price] as const,
  tags: ["stamp", "store", "tags"] as const,
  creator: (userId: string) => ["stamp", "store", "creator", userId] as const,
  pack: (packId: string) => ["stamp", "pack", packId] as const,
  stamp: (stampId: string) => ["stamp", "stamp", stampId] as const,
  library: ["stamp", "library"] as const,
  libraryHome: ["stamp", "library", "home"] as const,
  installed: ["stamp", "library", "installed"] as const,
  purchased: ["stamp", "library", "purchased"] as const,
  favorites: ["stamp", "library", "favorites"] as const,
  recents: (order: stamp.RecentOrder) => ["stamp", "library", "recents", order] as const,
  personal: ["stamp", "library", "personal"] as const,
  librarySearch: (query: string) => ["stamp", "library", "search", query] as const,
  usageLog: ["stamp", "library", "log"] as const,
  studio: ["stamp", "studio"] as const,
  creatorProfile: ["stamp", "studio", "profile"] as const,
  myPacks: (official: boolean) => ["stamp", "studio", "packs", official] as const,
  packStats: (packId: string, days: number) => ["stamp", "studio", "stats", packId, days] as const,
  review: ["stamp", "review"] as const,
  reviewQueue: ["stamp", "review", "queue"] as const,
};
