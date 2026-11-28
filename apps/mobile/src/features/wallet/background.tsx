import { useQueryClient } from "@tanstack/react-query";
import { rereadOnWelcome } from "@tokuly/core/query";
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
    const client = realtime();
    const off = client.onFrame((frame) => {
      if (frame.body.case === "wallet") applyWalletEvent(queryClient, frame.body.value);
    });
    // Pushes sent before the connection was up, or while it was down, are lost: ask again.
    const offWelcome = rereadOnWelcome(queryClient, client, [walletKeys.all]);
    return () => {
      off();
      offWelcome();
    };
  }, [queryClient]);
  return null;
}
