import { useQuery, useQueryClient } from "@tanstack/react-query";
import { walletQueries } from "@tokuly/core/wallet";
import { walletClient } from "./api";

export interface BalanceState {
  /** Spendable coins; `undefined` until the first answer arrives. */
  coins: bigint | undefined;
  /** Coins earned from other people (payable out, not spendable); `undefined` until loaded. */
  earnings: bigint | undefined;
  isPending: boolean;
  isError: boolean;
  /** Asks the server again. Rarely needed: spending and receiving coins is pushed live. */
  refresh(): Promise<void>;
}

/**
 * The signed-in user's balance, kept current while mounted: read once from
 * the API, then updated by every wallet RPC response and by the realtime
 * `WalletEvent` frames (see `background.tsx`).
 */
export function useBalance(): BalanceState {
  const queryClient = useQueryClient();
  const query = useQuery(walletQueries.balance(walletClient, queryClient));
  return {
    coins: query.data?.coins,
    earnings: query.data?.earnings,
    isPending: query.isPending,
    isError: query.isError,
    refresh: async () => {
      await query.refetch();
    },
  };
}
