import { toMillis } from "@tokuly/api";
import { useEffect } from "react";
import type { BadgeSources } from "#/seams/badges";
import { useDailyBonus } from "./data";

/** How soon the bonus is asked again when the server still says no after it unlocked (the phone's clock is ahead). */
export const RECHECK_MS = 20_000;

/**
 * Today's login bonus, while it waits to be claimed. The next one unlocks at
 * the account's midnight, which the server names: the bonus is asked again
 * then, once, not polled.
 */
function useBonusWaiting(): number {
  const { data: bonus, dataUpdatedAt, refetch } = useDailyBonus();
  const unlocksAt = bonus && !bonus.claimable ? toMillis(bonus.nextClaimAt) : 0;
  // biome-ignore lint/correctness/useExhaustiveDependencies: every answer that still says no sets the next ask
  useEffect(() => {
    if (unlocksAt <= 0) return;
    const wait = unlocksAt - Date.now();
    const timer = setTimeout(() => void refetch(), wait > 0 ? wait : RECHECK_MS);
    return () => clearTimeout(timer);
  }, [unlocksAt, dataUpdatedAt, refetch]);
  return bonus?.claimable ? 1 : 0;
}

export default { wallet: useBonusWaiting } satisfies BadgeSources;
