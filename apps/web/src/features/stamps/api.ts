import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { stamp } from "@tokuly/api";
import {
  markUninstalled,
  patchPack,
  patchStamp,
  refreshHoldings,
  stampKeys,
  stampQueries,
} from "@tokuly/core/stamps";
import { toast } from "sonner";
import { api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import i18n from "#/lib/i18n";

export const storeClient = () => api().service(stamp.StampStoreService);
export const libraryClient = () => api().service(stamp.StampLibraryService);
export const studioClient = () => api().service(stamp.StampStudioService);
export const reviewClient = () => api().service(stamp.StampReviewService);

// ───────────────────────────── Library ─────────────────────────────

/** Everything a picker needs in one call: installed packs, recents, favourites, personal stamps. */
export function useLibrary(options?: { refetchOnMount?: "always" }) {
  return useQuery({
    ...stampQueries.library(libraryClient),
    refetchOnMount: options?.refetchOnMount,
  });
}

/** A pack with its stamps (UUID or short ID). */
export function usePack(packId: string | undefined) {
  return useQuery({ ...stampQueries.pack(storeClient, packId ?? ""), enabled: Boolean(packId) });
}

/** Adds a (free or already bought) pack to the library. */
export function useInstallPack() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (packId: string) => libraryClient().installPack({ packId }),
    onSuccess: (response) => {
      const installed = response.pack;
      if (installed) patchPack(queryClient, installed.id, () => installed);
      toast.success(i18n.t("toast.installed", { ns: "stamps" }));
      return refreshHoldings(queryClient);
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "stamps" })),
  });
}

/** Removes a pack from the library (a purchase stays valid). */
export function useUninstallPack() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (packId: string) => libraryClient().uninstallPack({ packId }),
    onSuccess: (_, packId) => {
      markUninstalled(queryClient, packId);
      toast.success(i18n.t("toast.uninstalled", { ns: "stamps" }));
      return refreshHoldings(queryClient);
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "stamps" })),
  });
}

/** Flips a stamp's favourite mark; the star changes at once and rolls back if the server says no. */
export function useToggleFavorite() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({ stampId, favorited }: { stampId: string; favorited: boolean }) => {
      if (favorited) await libraryClient().favoriteStamp({ stampId });
      else await libraryClient().unfavoriteStamp({ stampId });
    },
    onMutate: ({ stampId, favorited }) => patchStamp(queryClient, stampId, { favorited }),
    onError: (error, { stampId, favorited }) => {
      patchStamp(queryClient, stampId, { favorited: !favorited });
      toast.error(errorMessage(error, { namespace: "stamps" }));
    },
    onSettled: () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: stampKeys.favorites }),
        queryClient.invalidateQueries({ queryKey: stampKeys.libraryHome }),
      ]),
  });
}

// ───────────────────────────── Formatting ─────────────────────────────

/** A count for humans: "1.2万" / "12K". */
export function formatCount(value: bigint | number): string {
  return new Intl.NumberFormat(i18n.language, {
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(value);
}
