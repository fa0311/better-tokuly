import { useQueryClient } from "@tanstack/react-query";
import { applyWalletEvent, walletKeys } from "@tokuly/core/wallet";
import { useEffect } from "react";
import { realtime } from "#/lib/realtime";

/**
 * Keeps the wallet live: balance changes, purchase results and payout
 * decisions pushed over the realtime connection go into the cache (the
 * balance with the highest revision wins). Pushes sent while there was no
 * connection are lost, so the balance is read again on every connection
 * and everything else after a gap.
 */
export default function WalletBackground() {
  const queryClient = useQueryClient();
  useEffect(() => {
    let welcomed = false;
    return realtime().onFrame((frame) => {
      if (frame.body.case === "wallet") applyWalletEvent(queryClient, frame.body.value);
      else if (frame.body.case === "welcome") {
        void queryClient.invalidateQueries({
          queryKey: welcomed ? walletKeys.all : walletKeys.balance,
        });
        welcomed = true;
      }
    });
  }, [queryClient]);
  return null;
}
