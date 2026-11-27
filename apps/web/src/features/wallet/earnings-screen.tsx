import { keepPreviousData, useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { fromDate, toDate, wallet } from "@tokuly/api";
import { formatCoins, formatMoney, uniqueById, walletKeys } from "@tokuly/core/wallet";
import { ChevronRight, Send } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { Card } from "#/components/ui/card";
import { Coins } from "#/components/ui/coins";
import { ConfirmDialog } from "#/components/ui/confirm";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { earningsClient, PAGE_SIZE } from "./api";
import { EarningsChart } from "./earnings-chart";
import { payoutMoney } from "./format";
import { SectionTitle, Stat } from "./parts";
import {
  isOpenPayout,
  PayoutAmount,
  PayoutRequestDialog,
  PayoutStatusChip,
  payoutMethodKey,
  useCancelPayout,
} from "./payouts";

type Range = "7d" | "30d" | "12m";

/** The period a range covers, ending now, cut on local day (or month) boundaries so buckets are whole. */
function rangeRequest(range: Range): { from: Date; bucket: wallet.EarningsBucket } {
  const now = new Date();
  if (range === "12m")
    return {
      from: new Date(now.getFullYear(), now.getMonth() - 11, 1),
      bucket: wallet.EarningsBucket.MONTH,
    };
  const days = range === "7d" ? 7 : 30;
  return {
    from: new Date(now.getFullYear(), now.getMonth(), now.getDate() - (days - 1)),
    bucket: wallet.EarningsBucket.DAY,
  };
}

/** Payout requests of the signed-in user, newest first. */
function usePayoutRequests() {
  return useInfiniteQuery({
    queryKey: walletKeys.payouts,
    queryFn: ({ pageParam }) =>
      earningsClient().listPayoutRequests({ page: { limit: PAGE_SIZE, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
}

/** The creator side of the wallet: what was earned, and getting it paid out. */
export function EarningsScreen() {
  const { t } = useTranslation(["wallet", "common"]);
  const [range, setRange] = useState<Range>("30d");
  const summary = useQuery({
    queryKey: walletKeys.earningsSummary(range),
    queryFn: () => {
      const { from, bucket } = rangeRequest(range);
      return earningsClient().getEarningsSummary({ from: fromDate(from), bucket });
    },
    // Switching the period keeps the previous numbers on screen (dimmed) instead of flashing a skeleton.
    placeholderData: keepPreviousData,
  });
  const payouts = usePayoutRequests();
  const requests = uniqueById(payouts.data?.pages.flatMap((page) => page.payouts) ?? []);
  const open = requests.find(isOpenPayout);

  if (summary.isError && !summary.data) {
    return (
      <Page title={t("earnings.title")} back="/wallet">
        <EmptyState
          emoji="😵‍💫"
          title={errorMessage(summary.error)}
          action={
            <Button variant="secondary" onClick={() => void summary.refetch()}>
              {t("common:actions.retry")}
            </Button>
          }
        />
      </Page>
    );
  }

  return (
    <Page title={t("earnings.title")} back="/wallet" wide>
      <div className="grid items-start gap-5 lg:grid-cols-[minmax(0,5fr)_minmax(0,7fr)]">
        <div className="flex min-w-0 flex-col gap-5">
          {summary.data ? (
            <EarningsHero
              summary={summary.data}
              hasOpenRequest={Boolean(open)}
              requestsLoaded={!payouts.isPending}
            />
          ) : (
            <Skeleton className="h-72 rounded-card" />
          )}
          <section>
            <SectionTitle>{t("earnings.requests")}</SectionTitle>
            {payouts.isPending ? (
              <Skeleton className="mt-1 h-24 rounded-card" />
            ) : requests.length > 0 ? (
              <>
                <ul className="mt-1 flex flex-col gap-3">
                  {requests.map((payout) => (
                    <li key={payout.id}>
                      <PayoutCard payout={payout} />
                    </li>
                  ))}
                </ul>
                <LoadMore query={payouts} />
              </>
            ) : (
              <Card className="mt-1 p-0">
                <EmptyState
                  emoji="🏦"
                  title={t("earnings.requestsEmpty")}
                  body={t("earnings.requestsEmptyBody")}
                />
              </Card>
            )}
          </section>
        </div>

        <section className="min-w-0">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <h2 className="px-1 text-sm font-black text-ink-soft">{t("earnings.period")}</h2>
            <Tabs<Range>
              value={range}
              onChange={setRange}
              items={[
                { value: "7d", label: t("earnings.ranges.7d") },
                { value: "30d", label: t("earnings.ranges.30d") },
                { value: "12m", label: t("earnings.ranges.12m") },
              ]}
            />
          </div>
          {summary.data ? (
            <div
              className={cn(
                "mt-3 flex flex-col gap-5 transition-opacity",
                summary.isPlaceholderData && "opacity-50",
              )}
            >
              <PeriodTotals summary={summary.data} />
              <EarningsChart
                series={summary.data.series}
                bucket={range === "12m" ? "month" : "day"}
              />
              <ByReason reasons={summary.data.byReason} />
            </div>
          ) : (
            <Skeleton className="mt-3 h-96 rounded-card" />
          )}
        </section>
      </div>
    </Page>
  );
}

/** What can be paid out right now, and the way to ask for it. */
function EarningsHero({
  summary,
  hasOpenRequest,
  requestsLoaded,
}: {
  summary: wallet.GetEarningsSummaryResponse;
  hasOpenRequest: boolean;
  requestsLoaded: boolean;
}) {
  const { t, i18n } = useTranslation("wallet");
  const [dialog, setDialog] = useState<{ attempt: number; open: boolean }>({
    attempt: 0,
    open: false,
  });
  const terms = summary.payoutTerms;
  const minimum = terms?.minCoins ?? 0n;
  const short = minimum - summary.available;
  const canRequest = Boolean(terms) && short <= 0n && summary.available > 0n && !hasOpenRequest;

  return (
    <section
      className="sticker-lg rounded-card bg-surface p-5"
      aria-label={t("earnings.available")}
    >
      <p className="text-sm font-black text-ink-soft">{t("earnings.available")}</p>
      <div className="mt-1 flex flex-wrap items-baseline gap-x-3">
        <Coins
          amount={summary.available}
          size="lg"
          className="text-5xl [&>span]:size-11 [&>span]:self-center [&>span]:text-2xl"
        />
        {terms && (
          <span className="text-lg font-black text-ink-soft">
            ≈{" "}
            {formatMoney(
              payoutMoney(summary.available, terms.minorPer100Coins),
              terms.currency,
              i18n.language,
            )}
          </span>
        )}
      </div>

      <dl className="mt-5 grid grid-cols-3 gap-3 border-t-2 border-line pt-4">
        <Stat label={t("earnings.pendingPayout")}>{formatCoins(summary.pendingPayout)}</Stat>
        <Stat label={t("earnings.lifetimeEarned")}>{formatCoins(summary.lifetimeEarned)}</Stat>
        <Stat label={t("earnings.lifetimePaidOut")}>{formatCoins(summary.lifetimePaidOut)}</Stat>
      </dl>

      <div className="mt-5">
        <Button
          block
          size="lg"
          icon={<Send className="size-5" />}
          disabled={!canRequest || !requestsLoaded}
          onClick={() => setDialog((previous) => ({ attempt: previous.attempt + 1, open: true }))}
        >
          {t("earnings.request")}
        </Button>
        {/* Why the button is off, in words; never a dead button without a reason. */}
        {hasOpenRequest ? (
          <p className="mt-2 text-center text-sm font-bold text-ink-soft">
            {t("earnings.blockedOpen")}
          </p>
        ) : short > 0n ? (
          <div className="mt-3">
            <div
              className="h-3 overflow-hidden rounded-full border-2 border-ink bg-surface-2"
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={Number(minimum)}
              aria-valuenow={Number(summary.available)}
              aria-label={t("earnings.progress")}
            >
              <div
                className="h-full bg-lime"
                style={{
                  width: `${minimum > 0n ? Math.min(100, Number((summary.available * 100n) / minimum)) : 0}%`,
                }}
              />
            </div>
            <p className="mt-2 text-center text-sm font-bold text-ink-soft">
              {t("earnings.blockedMinimum", { coins: formatCoins(short) })}
            </p>
          </div>
        ) : null}
      </div>

      {terms && (
        <p className="mt-4 text-center text-xs font-bold text-balance text-ink-faint">
          {t("earnings.terms", {
            minimum: formatCoins(terms.minCoins),
            rate: formatMoney(terms.minorPer100Coins, terms.currency, i18n.language),
            fee: terms.platformFeePercent,
          })}
        </p>
      )}

      {terms && dialog.attempt > 0 && (
        <PayoutRequestDialog
          key={dialog.attempt}
          open={dialog.open}
          onOpenChange={(open) => setDialog((previous) => ({ ...previous, open }))}
          available={summary.available}
          terms={terms}
        />
      )}
    </section>
  );
}

/** One payout request in the list: its state, the amounts, the reviewer's word, and the way to withdraw it. */
function PayoutCard({ payout }: { payout: wallet.PayoutRequest }) {
  const { t, i18n } = useTranslation("wallet");
  const [confirming, setConfirming] = useState(false);
  const cancel = useCancelPayout();

  return (
    <Card className="p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <PayoutAmount payout={payout} />
          <p className="mt-0.5 truncate text-sm text-ink-soft">
            {toDate(payout.createdAt)?.toLocaleDateString(i18n.language, { dateStyle: "medium" })}
            {" · "}
            {t(payoutMethodKey(payout.method))}
          </p>
        </div>
        <PayoutStatusChip status={payout.status} />
      </div>
      {payout.reviewNote && (
        <p className="mt-3 rounded-2xl bg-surface-2 px-3 py-2 text-sm leading-relaxed">
          <span className="font-black">
            {payout.status === wallet.PayoutStatus.REJECTED
              ? t("payout.rejectedReason")
              : t("payout.reviewNote")}
          </span>
          <span className="whitespace-pre-wrap break-words"> {payout.reviewNote}</span>
        </p>
      )}
      <div className="mt-3 flex items-center justify-between gap-2">
        {payout.status === wallet.PayoutStatus.PENDING ? (
          <Button variant="secondary" size="sm" onClick={() => setConfirming(true)}>
            {t("payout.cancel")}
          </Button>
        ) : (
          <span />
        )}
        <Link
          to="/wallet/payouts/$payoutId"
          params={{ payoutId: payout.id }}
          className="flex items-center text-sm font-black text-brand"
        >
          {t("payout.details")}
          <ChevronRight className="size-4" />
        </Link>
      </div>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t("payout.cancelConfirm.title")}
        description={t("payout.cancelConfirm.body", { coins: formatCoins(payout.coins) })}
        confirmLabel={t("payout.cancelConfirm.confirm")}
        danger
        loading={cancel.isPending}
        onConfirm={() => cancel.mutate(payout.id, { onSettled: () => setConfirming(false) })}
      />
    </Card>
  );
}

/** The totals of the chosen period. */
function PeriodTotals({ summary }: { summary: wallet.GetEarningsSummaryResponse }) {
  const { t } = useTranslation("wallet");
  const totals = summary.period;
  const fee = summary.payoutTerms?.platformFeePercent;
  return (
    <Card className="p-4">
      <dl className="grid grid-cols-3 gap-x-3 gap-y-4 sm:grid-cols-4">
        <Stat label={t("earnings.totals.net")} className="col-span-3 sm:col-span-1">
          <Coins amount={totals?.net ?? 0n} className="text-2xl" />
        </Stat>
        <Stat label={t("earnings.totals.gross")}>{formatCoins(totals?.gross ?? 0n)}</Stat>
        <Stat
          label={
            fee === undefined
              ? t("earnings.totals.fee")
              : t("earnings.totals.feePercent", { percent: fee })
          }
        >
          {formatCoins(totals?.fee ?? 0n)}
        </Stat>
        <Stat label={t("earnings.totals.count")}>
          {(totals?.count ?? 0).toLocaleString("en-US")}
        </Stat>
      </dl>
    </Card>
  );
}

/** Where the period's earnings came from, largest first, as ranked bars on one scale. */
function ByReason({ reasons }: { reasons: wallet.EarningsByReason[] }) {
  const { t } = useTranslation("wallet");
  const largest = reasons.reduce(
    (max, reason) => (reason.totals && reason.totals.net > max ? reason.totals.net : max),
    0n,
  );
  return (
    <Card className="p-4">
      <h3 className="font-black">{t("earnings.byReason")}</h3>
      {reasons.length > 0 ? (
        <ul className="mt-3 flex flex-col gap-3">
          {reasons.map((reason) => {
            const net = reason.totals?.net ?? 0n;
            return (
              <li key={reason.reason}>
                <div className="flex items-baseline justify-between gap-3">
                  <span className="min-w-0 truncate font-bold">
                    {reason.label || reason.reason}
                  </span>
                  <span className="shrink-0 text-sm font-bold text-ink-soft">
                    {t("earnings.reasonCount", { count: reason.totals?.count ?? 0 })}
                  </span>
                </div>
                <div className="mt-1 flex items-center gap-3">
                  <div className="h-3 min-w-0 flex-1 border-l border-ink-faint">
                    <div
                      className="h-full rounded-r-[4px] bg-brand"
                      style={{
                        width: `${largest > 0n ? Math.max(1, Number((net * 100n) / largest)) : 0}%`,
                      }}
                    />
                  </div>
                  <span className="w-16 shrink-0 text-right font-black tabular-nums">
                    {formatCoins(net)}
                  </span>
                </div>
              </li>
            );
          })}
        </ul>
      ) : (
        <p className="mt-2 text-sm leading-relaxed text-ink-soft">{t("earnings.byReasonEmpty")}</p>
      )}
    </Card>
  );
}
