import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { geo } from "@tokuly/api";
import { footprintKeys, footprintQueries } from "@tokuly/core/footprints";
import { currentAccount, isActive } from "#/features/session/account";
import { api } from "#/lib/api";

/**
 * Footprints in the query cache. The keys, query options and the game's rules
 * are `@tokuly/core/footprints`; this file wires them to the phone's API
 * client. Every client is `api()` when called: whoever is signed in now.
 * Only the account's own numbers are read here, as the server returns them;
 * they live only in the query cache, which starts over with every switch.
 */
export const footprintClient = () => api().service(geo.FootprintService);

/** Strings of this area (`@tokuly/i18n`). */
export const NS = "footprints";

export function useFootprintStats() {
  return useQuery(footprintQueries.stats(footprintClient));
}

export function useLeaderboard(regionId: string, scope: geo.LeaderboardScope) {
  return useQuery(footprintQueries.leaderboard(footprintClient, regionId, scope));
}

export function useRecap() {
  return useQuery(footprintQueries.recap(footprintClient));
}

/**
 * Marks the recap as seen, as the account that read it: the next one starts
 * from now. Sent only while that account is still the one signed in.
 */
export function useMarkRecapSeen() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async () => {
      const account = currentAccount();
      if (!account) return false;
      await footprintClient().getRecap({ markSeen: true });
      return isActive(account);
    },
    onSuccess: (active) => {
      if (active) void queryClient.invalidateQueries({ queryKey: footprintKeys.recap });
    },
  });
}
