import { useIsMutating, useMutation, useQueryClient } from "@tanstack/react-query";
import { wallet } from "@tokuly/api";
import { rememberBalance, walletKeys } from "@tokuly/core/wallet";
import { errorMessage } from "#/lib/errors";
import { asAccount } from "./data";

/** The claim in the mutation cache of the account. */
const claimKey = [...walletKeys.dailyBonus, "claim"] as const;

/**
 * Claiming today's login bonus. One request is on its way at a time,
 * however often the button is tapped and on however many screens the wallet
 * is open: the claim lives in the mutation cache, where each of them finds
 * it. (The server grants one claim a day either way, but its refusal of a
 * second request would be shown over a claim that went through.) Coins are
 * never added up here: the balance is the one the answer carries, and after
 * a failure — which may be an answer lost to the background or a dropped
 * connection rather than a refusal — the bonus and the balance are read
 * again from the server, and nothing is shown as won.
 */
export function useClaimBonus(onWon: (coins: bigint) => void, onError: (message: string) => void) {
  const queryClient = useQueryClient();
  const claiming = useIsMutating({ mutationKey: claimKey }) > 0;
  const claim = useMutation({
    mutationKey: claimKey,
    mutationFn: () =>
      asAccount((account) => account.api.service(wallet.WalletService).claimDailyBonus({})),
    onSuccess: (response) => {
      if (!response) return;
      if (response.bonus) queryClient.setQueryData(walletKeys.dailyBonus, response.bonus);
      rememberBalance(queryClient, response.balance);
      onWon(response.coins);
    },
    onError: (error) => {
      onError(errorMessage(error, { namespace: "wallet" }));
      void queryClient.invalidateQueries({ queryKey: walletKeys.dailyBonus });
      void queryClient.invalidateQueries({ queryKey: walletKeys.balance });
    },
  });
  return {
    claim: () => {
      if (queryClient.isMutating({ mutationKey: claimKey }) === 0) claim.mutate();
    },
    claiming,
  };
}

/**
 * Choosing one's own monthly limit (`null` removes it, from 18 on). The
 * screen shows the limit the server answers with, never the one asked for.
 */
export function useSetSpendingLimit(
  onSaved: (limitYen: bigint | null) => void,
  onError: (message: string) => void,
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (limitYen: bigint | null) =>
      asAccount((account) =>
        account.api
          .service(wallet.WalletService)
          .setSpendingLimit({ limitYen: limitYen ?? undefined }),
      ),
    onSuccess: (response, limitYen) => {
      if (!response) return;
      if (response.limit) queryClient.setQueryData(walletKeys.spendingLimit, response.limit);
      onSaved(limitYen);
    },
    onError: (error) => onError(errorMessage(error, { namespace: "wallet" })),
  });
}
