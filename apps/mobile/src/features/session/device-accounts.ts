import { useQuery } from "@tanstack/react-query";
import { accountKeys } from "@tokuly/core/session";
import { authClient } from "#/lib/api";
import { keepParked, parkedTokens } from "./vault";

/**
 * The accounts on this device, the active one first. A native client hands
 * in the refresh tokens of its parked accounts; the ones the server no
 * longer knows (signed out elsewhere, a password change) are forgotten here.
 */
export function useDeviceAccounts() {
  return useQuery({
    queryKey: accountKeys.deviceAccounts,
    queryFn: async () => {
      const tokens = await parkedTokens();
      const response = await authClient().listDeviceAccounts({
        refreshTokens: Object.values(tokens),
      });
      await keepParked(new Set(response.accounts.filter((a) => !a.active).map((a) => a.sessionId)));
      return response;
    },
  });
}
