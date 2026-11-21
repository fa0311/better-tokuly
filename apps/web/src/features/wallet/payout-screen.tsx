import { useQuery } from "@tanstack/react-query";
import { toDate, wallet } from "@tokuly/api";
import { Check } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { Card } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { earningsClient, walletKeys } from "./api";
import { formatCoins } from "./format";
import { Fact, LinkButton } from "./parts";
import { PayoutAmount, PayoutStatusChip, payoutMethodKey, useCancelPayout } from "./payouts";

/** The API lists a user's requests but has no single-request lookup for them: walk the list. */
async function findPayout(payoutId: string): Promise<wallet.PayoutRequest | null> {
  let cursor = "";
  do {
    const page = await earningsClient().listPayoutRequests({ page: { limit: 100, cursor } });
    const found = page.payouts.find((payout) => payout.id === payoutId);
    if (found) return found;
    cursor = page.page?.nextCursor ?? "";
  } while (cursor);
  return null;
}

/** One payout request from start to finish (where the "payout approved / rejected / paid" notifications lead). */
export function PayoutScreen({ payoutId }: { payoutId: string }) {
  const { t, i18n } = useTranslation("wallet");
  const { data: payout, isPending } = useQuery({ queryKey: walletKeys.payout(payoutId), queryFn: () => findPayout(payoutId) });
  const [confirming, setConfirming] = useState(false);
  const cancel = useCancelPayout();
  const format = (value: Date | undefined) => value?.toLocaleString(i18n.language, { dateStyle: "medium", timeStyle: "short" });

  return (
    <Page title={t("payout.title")} back="/wallet/earnings">
      {isPending ? (
        <Skeleton className="h-80 rounded-card" />
      ) : !payout ? (
        <EmptyState
          emoji="🏦"
          title={t("payout.notFound")}
          action={
            <LinkButton to="/wallet/earnings" variant="secondary">
              {t("payout.toEarnings")}
            </LinkButton>
          }
        />
      ) : (
        <div className="flex flex-col gap-5">
          <Card>
            <div className="flex items-start justify-between gap-3">
              <PayoutAmount payout={payout} className="text-xl" />
              <PayoutStatusChip status={payout.status} />
            </div>
            <ol className="mt-5">
              <Step done title={t("payout.steps.requested")} time={format(toDate(payout.createdAt))} />
              {payout.status === wallet.PayoutStatus.CANCELLED ? (
                <Step done last title={t("payout.steps.cancelled")}>
                  {t("payout.steps.returned", { coins: formatCoins(payout.coins) })}
                </Step>
              ) : payout.status === wallet.PayoutStatus.REJECTED ? (
                <Step done last title={t("payout.steps.rejected")} time={format(toDate(payout.reviewedAt))}>
                  {payout.reviewNote && <Quote label={t("payout.rejectedReason")}>{payout.reviewNote}</Quote>}
                  <p className="mt-1">{t("payout.steps.returned", { coins: formatCoins(payout.coins) })}</p>
                </Step>
              ) : (
                <>
                  <Step done={payout.status !== wallet.PayoutStatus.PENDING} title={payout.status === wallet.PayoutStatus.PENDING ? t("payout.steps.reviewing") : t("payout.steps.approved")} time={format(toDate(payout.reviewedAt))}>
                    {payout.status === wallet.PayoutStatus.PENDING && t("payout.steps.reviewingBody")}
                    {payout.status !== wallet.PayoutStatus.PENDING && payout.reviewNote && <Quote label={t("payout.reviewNote")}>{payout.reviewNote}</Quote>}
                  </Step>
                  <Step done={payout.status === wallet.PayoutStatus.PAID} last title={payout.status === wallet.PayoutStatus.PAID ? t("payout.steps.paid") : t("payout.steps.paying")} time={format(toDate(payout.paidAt))}>
                    {payout.paymentReference && <Quote label={t("payout.paymentReference")}>{payout.paymentReference}</Quote>}
                  </Step>
                </>
              )}
            </ol>
            {payout.status === wallet.PayoutStatus.PENDING && (
              <Button variant="secondary" block className="mt-2" onClick={() => setConfirming(true)}>
                {t("payout.cancel")}
              </Button>
            )}
          </Card>

          <Card className="py-2">
            <dl className="divide-y-2 divide-line">
              <Fact label={t("payout.form.method")}>{t(payoutMethodKey(payout.method))}</Fact>
              {payout.destination && (
                <Fact label={t("payout.form.destination")}>
                  <span className="whitespace-pre-wrap">{payout.destination}</span>
                </Fact>
              )}
              {payout.note && (
                <Fact label={t("payout.form.note")}>
                  <span className="whitespace-pre-wrap">{payout.note}</span>
                </Fact>
              )}
              <Fact label={t("payout.id")}>
                <span className="break-all font-mono text-xs font-medium text-ink-soft">{payout.id}</span>
              </Fact>
            </dl>
          </Card>

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
        </div>
      )}
    </Page>
  );
}

/** One stage of a payout's journey: reached (ticked) or still ahead (hollow). */
function Step({ done, last, title, time, children }: { done?: boolean; last?: boolean; title: string; time?: string; children?: ReactNode }) {
  return (
    <li className="relative flex gap-3 pb-5 last:pb-0">
      {!last && <span aria-hidden className={cn("absolute top-7 bottom-0 left-[13px] w-0.5", done ? "bg-ink" : "bg-line")} />}
      <span className={cn("grid size-7 shrink-0 place-items-center rounded-full border-2", done ? "border-ink bg-lime text-[#17122b]" : "border-line bg-surface-2")}>
        {done && <Check className="size-4" strokeWidth={3.5} />}
      </span>
      <div className="min-w-0 flex-1 pt-0.5">
        <p className={cn("font-black", !done && "text-ink-soft")}>{title}</p>
        {time && <p className="text-sm text-ink-soft">{time}</p>}
        {children && <div className="mt-1 text-sm leading-relaxed text-ink-soft">{children}</div>}
      </div>
    </li>
  );
}

function Quote({ label, children }: { label: string; children: ReactNode }) {
  return (
    <p className="rounded-2xl bg-surface-2 px-3 py-2 text-ink">
      <span className="font-black">{label}</span> <span className="whitespace-pre-wrap break-words">{children}</span>
    </p>
  );
}
