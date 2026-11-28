import { useQueryClient } from "@tanstack/react-query";
import { rereadOnWelcome } from "@tokuly/core/query";
import { walletKeys } from "@tokuly/core/wallet/keys";
import { useEffect } from "react";
import { onFramesLoading, realtime } from "#/lib/realtime";
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
    const offEvents = onFramesLoading("wallet", () =>
      import("./frames").then(
        ({ applyWalletEvent }) =>
          (event) =>
            applyWalletEvent(queryClient, event),
      ),
    );
    // Pushes sent before the connection was up, or while it was down, are lost: ask again.
    const offWelcome = rereadOnWelcome(queryClient, client, [walletKeys.all]);
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
      offEvents();
      offWelcome();
      offSession();
    };
  }, [queryClient]);

  return null;
}

function signedInUserId(session: ReturnType<typeof useSessionStore.getState>): string | undefined {
  return session.state.status === "signedIn" ? session.state.me.user?.id : undefined;
}
