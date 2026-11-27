import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { wallet } from "@tokuly/api";
import {
  formatMoney,
  limitPresets,
  limitSummary,
  ownLimitProblem,
  resetDay,
  usedShare,
  walletKeys,
  walletQueries,
} from "@tokuly/core/wallet";
import type { TFunction } from "i18next";
import { ChevronRight, ShieldCheck } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Card } from "#/components/ui/card";
import { TextField } from "#/components/ui/field";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { errorMessage, reasonText } from "#/lib/errors";
import { walletClient } from "./api";
import { LinkButton, SectionTitle } from "./parts";

/** The monthly spending limit and this month's spending, as the server has them. */
export const useSpendingLimit = () => useQuery(walletQueries.spendingLimit(walletClient));

const yen = (amount: bigint, language: string) => formatMoney(amount, "JPY", language);

/** 「今月はあと ¥1,200 まで使えるよ」 and its siblings (`limitSummary`). */
function line(limit: wallet.SpendingLimit, t: TFunction<"wallet">, language: string): string {
  const { key, values } = limitSummary(limit, language);
  return t(key, values);
}

/**
 * What is left of the month's limit, with the way to change it. On the
 * wallet's front page and above the coin shop.
 */
export function SpendingLimitBanner({ className }: { className?: string }) {
  const { t, i18n } = useTranslation("wallet");
  const { data: limit } = useSpendingLimit();
  if (!limit) return null;
  const reached = limit.remainingYen !== undefined && limit.remainingYen <= 0n;
  return (
    <Card
      data-testid="spending-limit-banner"
      className={cn("flex items-center gap-3 p-3", reached && "bg-sun text-[#17122b]", className)}
    >
      <span className="grid size-10 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-lime text-[#17122b]">
        <ShieldCheck className="size-5" />
      </span>
      <p className="min-w-0 flex-1 text-sm font-black">{line(limit, t, i18n.language)}</p>
      <LinkButton to="/wallet/limit" variant="ghost" size="sm" className="shrink-0 px-2">
        {t("limit.change")}
        <ChevronRight className="size-4" />
      </LinkButton>
    </Card>
  );
}

/** `/wallet/limit`: this month's spending against the limit, and choosing one's own. */
export function SpendingLimitScreen() {
  const { t } = useTranslation(["wallet", "common"]);
  const { data: limit, isPending, isError, error, refetch } = useSpendingLimit();
  return (
    <Page title={t("limit.title")} back="/wallet">
      {isPending ? (
        <Skeleton className="h-48 rounded-card" />
      ) : isError || !limit ? (
        <EmptyState
          emoji="😵‍💫"
          title={errorMessage(error, { namespace: "wallet" })}
          action={
            <Button variant="secondary" onClick={() => void refetch()}>
              {t("common:actions.retry")}
            </Button>
          }
        />
      ) : (
        <div className="flex flex-col gap-5">
          <ThisMonth limit={limit} />
          <ChooseLimit limit={limit} />
        </div>
      )}
    </Page>
  );
}

function ThisMonth({ limit }: { limit: wallet.SpendingLimit }) {
  const { t, i18n } = useTranslation("wallet");
  const share = usedShare(limit);
  return (
    <Card className="flex flex-col gap-3 p-5">
      <p className="text-lg font-black" data-testid="spending-limit-summary">
        {line(limit, t, i18n.language)}
      </p>
      <div className="flex items-baseline justify-between gap-3">
        <span className="text-sm font-bold text-ink-soft">{t("limit.spent")}</span>
        <span className="text-2xl font-black tabular-nums">
          {yen(limit.spentYen, i18n.language)}
        </span>
      </div>
      {share !== null && limit.limitYen !== undefined && (
        <>
          <div
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round(share * 100)}
            className="h-4 overflow-hidden rounded-full border-2 border-ink bg-surface-2"
          >
            <div
              className={cn(
                "h-full",
                share >= 1 ? "bg-coral" : share >= 0.8 ? "bg-sun" : "bg-lime",
              )}
              style={{ width: `${share * 100}%` }}
            />
          </div>
          <div className="flex justify-between text-xs font-bold text-ink-soft">
            <span>{t("limit.of", { amount: yen(limit.limitYen, i18n.language) })}</span>
            <span>{t("limit.resets", { date: resetDay(limit, i18n.language) })}</span>
          </div>
        </>
      )}
      {limit.ceilingYen !== undefined && (
        <p className="rounded-2xl bg-surface-2 p-3 text-sm font-bold">
          {t("limit.ceiling", { amount: yen(limit.ceilingYen, i18n.language) })}
        </p>
      )}
      <p className="text-xs text-ink-faint">{t("limit.counts")}</p>
    </Card>
  );
}

/** One-tap amounts, any amount typed in, and (from 18) no limit at all. */
function ChooseLimit({ limit }: { limit: wallet.SpendingLimit }) {
  const { t, i18n } = useTranslation(["wallet", "common"]);
  const queryClient = useQueryClient();
  const [typed, setTyped] = useState("");
  const save = useMutation({
    mutationFn: (limitYen: bigint | null) =>
      walletClient().setSpendingLimit({ limitYen: limitYen ?? undefined }),
    onSuccess: (response, limitYen) => {
      if (response.limit) queryClient.setQueryData(walletKeys.spendingLimit, response.limit);
      setTyped("");
      toast.success(
        limitYen === null
          ? t("limit.cleared")
          : t("limit.saved", { amount: yen(limitYen, i18n.language) }),
      );
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "wallet" })),
  });

  const typedYen = /^\d{1,9}$/.test(typed) ? BigInt(typed) : null;
  const problem =
    typed === ""
      ? null
      : typedYen === null
        ? "SPENDING_LIMIT_INVALID"
        : ownLimitProblem(limit, typedYen);
  const canClear = limit.ownLimitYen !== undefined && ownLimitProblem(limit, null) === null;

  return (
    <section>
      <SectionTitle>{t("limit.choose")}</SectionTitle>
      <Card className="mt-1 flex flex-col gap-4 p-4">
        <p className="text-sm font-bold">
          {t("limit.own")}:{" "}
          <span data-testid="spending-limit-own">
            {limit.ownLimitYen === undefined
              ? t("limit.unlimited")
              : yen(limit.ownLimitYen, i18n.language)}
          </span>
        </p>
        <div className="flex flex-wrap gap-2">
          {limitPresets(limit).map((preset) => (
            <Button
              key={preset.toString()}
              variant={limit.ownLimitYen === preset ? "primary" : "secondary"}
              size="sm"
              disabled={save.isPending}
              onClick={() => save.mutate(preset)}
            >
              {yen(preset, i18n.language)}
            </Button>
          ))}
        </div>
        <form
          className="flex items-end gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            if (typedYen !== null && problem === null) save.mutate(typedYen);
          }}
        >
          <TextField
            className="flex-1"
            label={t("limit.custom")}
            inputMode="numeric"
            value={typed}
            onChange={(event) => setTyped(event.target.value.replace(/[^\d]/g, ""))}
            error={problem ? reasonText(problem, { namespace: "wallet" }) : undefined}
          />
          <Button
            type="submit"
            disabled={typedYen === null || problem !== null}
            loading={save.isPending}
          >
            {t("limit.save")}
          </Button>
        </form>
        {canClear && (
          <Button
            variant="ghost"
            className="self-start"
            disabled={save.isPending}
            onClick={() => save.mutate(null)}
          >
            {t("limit.clear")}
          </Button>
        )}
      </Card>
    </section>
  );
}
