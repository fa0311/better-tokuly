import { useMutation, useQueryClient } from "@tanstack/react-query";
import { wallet } from "@tokuly/api";
import { rememberBalance, walletKeys } from "@tokuly/core/wallet";
import { useRef } from "react";
import { errorMessage } from "#/lib/errors";
import { asAccount } from "./data";

/**
 * Claiming today's login bonus. One request is on its way at a time,
 * however often the button is tapped (the server grants one claim a day
 * either way). Coins are never added up here: the balance is the one the
 * answer carries, and after a failure — which may be an answer lost to
 * the background or a dropped connection rather than a refusal — the bonus
 * and the balance are read again from the server, and nothing is shown as
 * won.
 */
export function useClaimBonus(onWon: (coins: bigint) => void, onError: (message: string) => void) {
  const queryClient = useQueryClient();
  const sending = useRef(false);
  const claim = useMutation({
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
    onSettled: () => {
      sending.current = false;
    },
  });
  return {
    claim: () => {
      if (sending.current) return;
      sending.current = true;
      claim.mutate();
    },
    claiming: claim.isPending,
  };
}
