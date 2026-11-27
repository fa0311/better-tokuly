import { type QueryClient, useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { stamp } from "@tokuly/api";
import {
  markUninstalled,
  patchPack,
  refreshHoldings,
  type ShelfFilter,
  stampQueries,
} from "@tokuly/core/stamps";
import { currentAccount, isActive } from "#/features/session/account";
import { api } from "#/lib/api";

/**
 * Stamps in the query cache. The keys, query options and cache edits are
 * `@tokuly/core/stamps`; this file wires them to the phone's API client.
 * Every client is `api()` when called: whoever is signed in now.
 */
export const storeClient = () => api().service(stamp.StampStoreService);
export const libraryClient = () => api().service(stamp.StampLibraryService);

/** Strings of this area (`@tokuly/i18n`). */
export const NS = "stamps";

/** Store pages are this long. */
const SHELF_PAGE = 24;

/** The picker's library: installed packs, recents, favourites, personal stamps. Read again whenever it opens. */
export function useLibrary(enabled = true) {
  return useQuery({ ...stampQueries.library(libraryClient), enabled, refetchOnMount: "always" });
}

export function usePack(packId: string) {
  return useQuery({ ...stampQueries.pack(storeClient, packId), enabled: packId !== "" });
}

/** One stamp and its pack (a draft's stamp, a received stamp). */
export function useStamp(stampId: string) {
  return useQuery({ ...stampQueries.stamp(storeClient, stampId), enabled: stampId !== "" });
}

export function useStoreHome() {
  return useQuery(stampQueries.storeHome(storeClient));
}

export function useShelf(filter: ShelfFilter) {
  return useInfiniteQuery(stampQueries.shelf(storeClient, filter, SHELF_PAGE));
}

export function useInstalled() {
  return useInfiniteQuery(stampQueries.installed(libraryClient));
}

/**
 * Adds a free (or already bought) pack to the library, or takes it out, as
 * the account signed in now. Its answer belongs to that account: once
 * another one took over, nothing of it reaches the cache and no error is
 * shown (README, "Accounts"). `true` when it went through for the person
 * still here.
 */
export async function setInstalled(
  queryClient: QueryClient,
  packId: string,
  installed: boolean,
): Promise<boolean> {
  const account = currentAccount();
  if (!account) return false;
  const client = account.api.service(stamp.StampLibraryService);
  try {
    if (installed) {
      const { pack } = await client.installPack({ packId });
      if (!isActive(account)) return false;
      if (pack) patchPack(queryClient, pack.id, () => pack);
    } else {
      await client.uninstallPack({ packId });
      if (!isActive(account)) return false;
      markUninstalled(queryClient, packId);
    }
  } catch (error) {
    if (isActive(account)) throw error;
    return false;
  }
  void refreshHoldings(queryClient);
  return true;
}
