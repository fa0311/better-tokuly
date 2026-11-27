import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { errorReason, toMillis, type wallet } from "@tokuly/api";
import {
  collectedSteps,
  countdown,
  rememberBalance,
  walletKeys,
  walletQueries,
} from "@tokuly/core/wallet";
import { Check, Flame, Gift } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Card } from "#/components/ui/card";
import { CoinIcon, Coins } from "#/components/ui/coins";
import { Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { walletClient } from "./api";
import { CoinBurst, useCoinBurst } from "./celebration";

/** The time now, refreshed every `everyMs` while mounted (for countdowns). */
function useNow(everyMs: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), everyMs);
    return () => clearInterval(timer);
  }, [everyMs]);
  return now;
}

/**
 * The daily login bonus: the reward ladder with the streak so far and the
 * claim, which is the one moment of the wallet that should feel like a prize.
 * Renders nothing on servers where the bonus is switched off.
 */
export function DailyBonusCard() {
  const { t } = useTranslation("wallet");
  const queryClient = useQueryClient();
  const { burst, fire } = useCoinBurst();
  /** Coins of the claim made on this screen just now (drives the reward state). */
  const [won, setWon] = useState<bigint | null>(null);

  const { data: bonus, isPending } = useQuery(walletQueries.dailyBonus(walletClient));

  const claim = useMutation({
    mutationFn: () => walletClient().claimDailyBonus({}),
    onSuccess: (response) => {
      if (response.bonus) queryClient.setQueryData(walletKeys.dailyBonus, response.bonus);
      rememberBalance(queryClient, response.balance);
      setWon(response.coins);
      fire();
    },
    onError: (error) => {
      toast.error(errorMessage(error));
      // Claimed elsewhere a moment ago, or switched off: show the real state.
      if (errorReason(error))
        void queryClient.invalidateQueries({ queryKey: walletKeys.dailyBonus });
    },
  });

  if (isPending) return <Skeleton className="h-56 rounded-card" />;
  if (!bonus || bonus.cycle.length === 0) return null;

  const collected = collectedSteps(bonus);
  return (
    <Card className="relative p-4" role="region" aria-label={t("bonus.title")}>
      <div className="flex items-center gap-3">
        <span
          className={cn(
            "sticker-sm grid size-12 shrink-0 place-items-center rounded-2xl bg-sun text-[#17122b]",
            bonus.claimable && "animate-wiggle",
          )}
        >
          <Gift className="size-6" />
        </span>
        <div className="min-w-0 flex-1">
          <h2 className="font-black">{t("bonus.title")}</h2>
          <p className="text-sm leading-snug text-ink-soft">
            {bonus.claimable ? t("bonus.leadClaimable") : t("bonus.leadClaimed")}
          </p>
        </div>
        {bonus.streak > 0 && (
          <span className="sticker-sm inline-flex h-8 shrink-0 items-center gap-1 rounded-full bg-coral px-3 text-sm font-black text-[#17122b]">
            <Flame className="size-4" />
            {t("bonus.streak", { count: bonus.streak })}
          </span>
        )}
      </div>

      <Ladder bonus={bonus} collected={collected} />

      <div className="relative mt-4">
        {bonus.claimable ? (
          <Button
            variant="lime"
            size="lg"
            block
            loading={claim.isPending}
            onClick={() => claim.mutate()}
          >
            {t("bonus.claim")}
            <Coins amount={bonus.nextCoins} signed />
          </Button>
        ) : (
          <Claimed bonus={bonus} won={won} />
        )}
        <CoinBurst burst={burst} />
      </div>

      {bonus.totalClaims > 0 && (
        <p className="mt-3 text-center text-xs font-bold text-ink-faint">
          {t("bonus.record", { longest: bonus.longestStreak, total: bonus.totalClaims })}
        </p>
      )}
    </Card>
  );
}

/** The reward ladder: one tile per streak day, collected ones ticked, the next one highlighted. */
function Ladder({ bonus, collected }: { bonus: wallet.DailyBonus; collected: number }) {
  const { t } = useTranslation("wallet");
  const columns = Math.min(bonus.cycle.length, 7);
  return (
    <ol
      className="mt-4 grid gap-1.5"
      style={{ gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))` }}
      aria-label={t("bonus.ladder")}
    >
      {bonus.cycle.map((coins, index) => {
        const day = index + 1;
        const done = day <= collected;
        const next = bonus.claimable && day === bonus.nextCycleDay;
        return (
          <li
            // The ladder is a fixed sequence: the position is the identity.
            key={index}
            aria-current={next ? "step" : undefined}
            className={cn(
              "flex flex-col items-center gap-1 rounded-2xl border-2 px-0.5 py-2 text-center",
              done
                ? "border-ink bg-lime text-[#17122b]"
                : next
                  ? "sticker-sm bg-sun text-[#17122b]"
                  : "border-line bg-surface-2 text-ink-soft",
            )}
          >
            <span className="text-[10px] font-black leading-none">{t("bonus.day", { day })}</span>
            {done ? (
              <span
                className="grid size-5 place-items-center"
                role="img"
                aria-label={t("bonus.collected")}
              >
                <Check className="size-5" strokeWidth={4} />
              </span>
            ) : (
              <CoinIcon className={cn("size-5 text-[11px]", !next && "opacity-60")} />
            )}
            <span className="text-xs font-black leading-none tabular-nums">{coins.toString()}</span>
          </li>
        );
      })}
    </ol>
  );
}

/** After today's claim: what was won (when it happened here) and when the next one unlocks. */
function Claimed({ bonus, won }: { bonus: wallet.DailyBonus; won: bigint | null }) {
  const { t } = useTranslation("wallet");
  const queryClient = useQueryClient();
  const now = useNow(20_000);
  const unlocksAt = toMillis(bonus.nextClaimAt);
  const left = countdown(unlocksAt, now);
  const unlocked = unlocksAt > 0 && now >= unlocksAt;

  // Midnight passed with the card open: today's bonus is there to take.
  useEffect(() => {
    if (unlocked) void queryClient.invalidateQueries({ queryKey: walletKeys.dailyBonus });
  }, [unlocked, queryClient]);

  return (
    <div
      className="flex min-h-14 flex-col items-center justify-center gap-1 rounded-[1.75rem] border-2 border-dashed border-line px-4 py-2.5 text-center"
      role="status"
    >
      {won !== null ? (
        <p className="animate-pop-in flex items-center gap-2 text-lg font-black">
          {t("bonus.won")}
          <Coins
            amount={won}
            signed
            className="rounded-full bg-lime py-0.5 pr-2.5 pl-1.5 text-lg text-[#17122b]"
          />
        </p>
      ) : (
        <p className="flex items-center gap-1.5 font-black">
          <Check className="size-5" strokeWidth={3} />
          {t("bonus.claimed")}
        </p>
      )}
      {unlocksAt > 0 && (
        <p className="flex flex-wrap items-center justify-center gap-x-1.5 text-sm font-bold text-ink-soft">
          {left.hours > 0
            ? t("bonus.nextInHours", { hours: left.hours, minutes: left.minutes })
            : t("bonus.nextInMinutes", { minutes: left.minutes })}
          <Coins amount={bonus.nextCoins} signed size="sm" />
        </p>
      )}
    </div>
  );
}
