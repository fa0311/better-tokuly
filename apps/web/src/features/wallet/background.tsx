import { useQueryClient } from "@tanstack/react-query";
import { applyWalletEvent, walletKeys } from "@tokuly/core/wallet";
import { useEffect } from "react";
import { realtime } from "#/lib/realtime";
import { useSessionStore } from "#/stores/session";

/**
 * Keeps the wallet live on every page: balance changes, purchase results and
 * payout decisions pushed over the realtime connection go straight into the
 * query cache, so `useBalance()` and the wallet screens never poll.
 */
export default function WalletBackground() {
  const queryClient = useQueryClient();

  useEffect(() => {
    const client = realtime();
    let connectedBefore = client.connectionStatus === "connected";
    const offFrame = client.onFrame((frame) => {
      if (frame.body.case === "wallet") {
        applyWalletEvent(queryClient, frame.body.value);
      } else if (frame.body.case === "welcome") {
        // Pushes sent while there was no connection are lost: ask again. The
        // balance is cheap and matters most, so it is checked on every
        // connection; the lists only after a reconnect.
        void queryClient.invalidateQueries({
          queryKey: connectedBefore ? walletKeys.all : walletKeys.balance,
        });
        connectedBefore = true;
      }
    });
    client.start();

    // The cache belongs to one account: forget it when that account leaves,
    // so the next person to sign in here never meets someone else's coins
    // (a stale balance with a higher revision would even win over theirs).
    let userId = signedInUserId(useSessionStore.getState());
    const offSession = useSessionStore.subscribe((session) => {
      const next = signedInUserId(session);
      if (next === userId) return;
      if (userId) queryClient.removeQueries({ queryKey: walletKeys.all });
      userId = next;
    });

    return () => {
      offFrame();
      offSession();
    };
  }, [queryClient]);

  return null;
}

function signedInUserId(session: ReturnType<typeof useSessionStore.getState>): string | undefined {
  return session.state.status === "signedIn" ? session.state.me.user?.id : undefined;
}
