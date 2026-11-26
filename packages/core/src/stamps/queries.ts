import type { stamp } from "@tokuly/api";
import type { ClientSource } from "../query";
import { stampKeys } from "./keys";

type Store = ClientSource<typeof stamp.StampStoreService>;
type Library = ClientSource<typeof stamp.StampLibraryService>;

/** What a store list shows: a shelf, narrowed by tag and price, or a search. */
export interface ShelfFilter {
  section: stamp.StoreSection;
  tag: string;
  price: stamp.PriceFilter;
  /** Set: search the store for it instead of listing the shelf. */
  query?: string;
}

/** Query options of the stamp domain. Each app adds `enabled` and its hooks around them. */
export const stampQueries = {
  /** The store's landing page: featured, popular and new packs, and the most used tags. */
  storeHome: (client: Store) => ({
    queryKey: stampKeys.storeHome,
    queryFn: () => client().getStoreHome({}),
  }),

  /** One shelf of the store, or a search, page by page. */
  shelf: (client: Store, filter: ShelfFilter, pageSize: number) => ({
    queryKey: filter.query
      ? stampKeys.storeSearch(filter.query, filter.price)
      : stampKeys.storeList(filter.section, filter.tag, filter.price),
    queryFn: async ({ pageParam }: { pageParam: string }) => {
      const page = { limit: pageSize, cursor: pageParam };
      const response = filter.query
        ? await client().searchPacks({ query: filter.query, price: filter.price, page })
        : await client().listStorePacks({
            section: filter.section,
            tag: filter.tag,
            price: filter.price,
            page,
          });
      return { packs: response.packs, next: response.page?.nextCursor ?? "" };
    },
    initialPageParam: "",
    getNextPageParam: (last: { next: string }) => last.next || undefined,
  }),

  /** The tags used by the most published packs. */
  tags: (client: Store) => ({
    queryKey: stampKeys.tags,
    queryFn: () => client().listPopularTags({ limit: 30 }),
    staleTime: 5 * 60_000,
  }),

  /** A pack with its stamps (UUID or short ID). */
  pack: (client: Store, packId: string) => ({
    queryKey: stampKeys.pack(packId),
    queryFn: async () => (await client().getPack({ packId })).pack!,
  }),

  /** One stamp and its pack: what a tap on a received stamp shows. */
  stamp: (client: Store, stampId: string) => ({
    queryKey: stampKeys.stamp(stampId),
    queryFn: () => client().getStamp({ stampId }),
  }),

  /** Everything a picker needs in one call: installed packs, recents, favourites, personal stamps. */
  library: (client: Library) => ({
    queryKey: stampKeys.libraryHome,
    queryFn: () => client().getLibrary({}),
  }),

  /** The installed packs in the user's order, page by page. */
  installed: (client: Library) => ({
    queryKey: stampKeys.installed,
    queryFn: ({ pageParam }: { pageParam: string }) =>
      client().listInstalledPacks({ page: { limit: 100, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last: stamp.ListInstalledPacksResponse) =>
      last.page?.nextCursor || undefined,
  }),
};
