import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toDate, toMillis, wallet } from "@tokuly/api";
import { BadgeCheck, Ban, Check, Copy, WalletCards } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextArea, TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { compactAge } from "#/lib/format";
import { PAGE_SIZE, walletAdminClient, walletKeys } from "../api";
import { uniqueById } from "../ledger";
import { Fact } from "../parts";
import { PayoutAmount, PayoutStatusChip, payoutMethodKey, payoutStatusKey } from "../payouts";
import { useIsWide } from "./hooks";
import { FilterChips, Pane, UserLine } from "./parts";

const FILTERS = [
  wallet.PayoutStatus.PENDING,
  wallet.PayoutStatus.APPROVED,
  wallet.PayoutStatus.PAID,
  wallet.PayoutStatus.REJECTED,
  wallet.PayoutStatus.CANCELLED,
];

const REVIEW_NOTE_MAX = 500;
const PAYMENT_REFERENCE_MAX = 200;

/**
 * The payout review queue, oldest first, next to the request being reviewed
 * (side by side on wide screens, a sheet on narrow ones).
 */
export function PayoutQueue({
  selectedId,
  onSelect,
  onOpenUser,
}: {
  selectedId?: string;
  onSelect(payoutId?: string): void;
  onOpenUser(userId: string): void;
}) {
  const { t } = useTranslation("wallet");
  const wide = useIsWide();
  const [status, setStatus] = useState(wallet.PayoutStatus.PENDING);
  const query = useInfiniteQuery({
    queryKey: walletKeys.admin.queue(status),
    queryFn: ({ pageParam }) =>
      walletAdminClient().listPayoutQueue({
        status,
        page: { limit: PAGE_SIZE, cursor: pageParam },
      }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const payouts = uniqueById(query.data?.pages.flatMap((page) => page.payouts) ?? []);

  return (
    <div className="grid items-start gap-4 lg:grid-cols-[minmax(0,5fr)_minmax(0,6fr)]">
      <div className="min-w-0">
        <FilterChips<wallet.PayoutStatus>
          label={t("staff.payouts.filter")}
          value={status}
          onChange={setStatus}
          options={FILTERS.map((value) => ({ value, label: t(payoutStatusKey(value)) }))}
        />
        <Pane
          className="mt-3"
          title={t("staff.payouts.listTitle", { status: t(payoutStatusKey(status)) })}
        >
          {query.isPending ? (
            <div className="flex flex-col gap-2 p-3">
              {[0, 1, 2].map((row) => (
                <Skeleton key={row} className="h-16" />
              ))}
            </div>
          ) : query.isError ? (
            <EmptyState emoji="😵‍💫" title={errorMessage(query.error)} />
          ) : payouts.length > 0 ? (
            <>
              <ul className="divide-y-2 divide-line">
                {payouts.map((payout) => (
                  <li key={payout.id}>
                    <button
                      type="button"
                      onClick={() => onSelect(payout.id)}
                      aria-current={payout.id === selectedId ? "true" : undefined}
                      className={cn(
                        "flex w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-surface-2",
                        payout.id === selectedId && "bg-brand-soft",
                      )}
                    >
                      <Avatar user={payout.user} size="sm" />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-sm font-black">
                          {payout.user ? payout.user.displayName : t("staff.deletedUser")}
                        </span>
                        <span className="block truncate text-xs text-ink-soft">
                          {payout.user ? `@${payout.user.handle} · ` : ""}
                          {t(payoutMethodKey(payout.method))} ·{" "}
                          {compactAge(toMillis(payout.createdAt))}
                        </span>
                      </span>
                      <PayoutAmount
                        payout={payout}
                        className="shrink-0 flex-col items-end text-sm"
                      />
                    </button>
                  </li>
                ))}
              </ul>
              <LoadMore query={query} />
            </>
          ) : (
            <EmptyState
              emoji={status === wallet.PayoutStatus.PENDING ? "🎉" : "🗂️"}
              title={
                status === wallet.PayoutStatus.PENDING
                  ? t("staff.payouts.emptyPending")
                  : t("staff.payouts.empty")
              }
            />
          )}
        </Pane>
      </div>

      {wide ? (
        <Pane className="sticky top-[4.5rem]" title={t("staff.payouts.detailTitle")}>
          {selectedId ? (
            <div className="p-4">
              <PayoutReview key={selectedId} payoutId={selectedId} onOpenUser={onOpenUser} />
            </div>
          ) : (
            <EmptyState emoji="👈" title={t("staff.payouts.pick")} />
          )}
        </Pane>
      ) : (
        <Dialog
          open={Boolean(selectedId)}
          onOpenChange={(open) => !open && onSelect(undefined)}
          title={t("staff.payouts.detailTitle")}
        >
          {selectedId && (
            <PayoutReview key={selectedId} payoutId={selectedId} onOpenUser={onOpenUser} />
          )}
        </Dialog>
      )}
    </div>
  );
}

type ReviewAction = "approve" | "reject" | "paid";

/** One payout request for the reviewer: who, how much, where to, and the decisions that are possible now. */
function PayoutReview({
  payoutId,
  onOpenUser,
}: {
  payoutId: string;
  onOpenUser(userId: string): void;
}) {
  const { t, i18n } = useTranslation(["wallet", "common"]);
  const queryClient = useQueryClient();
  // The decision being confirmed. It stays set while its dialog closes, so the dialog does not change face on the way out.
  const [action, setAction] = useState<ReviewAction>("approve");
  const [confirming, setConfirming] = useState(false);
  const [text, setText] = useState("");
  const [missing, setMissing] = useState(false);
  const {
    data: payout,
    isPending,
    error,
  } = useQuery({
    queryKey: walletKeys.admin.payout(payoutId),
    queryFn: async () => (await walletAdminClient().getPayoutRequest({ payoutId })).payout ?? null,
  });

  const review = useMutation({
    mutationFn: async (kind: ReviewAction) => {
      const note = text.trim();
      if (kind === "approve")
        return (await walletAdminClient().approvePayout({ payoutId, reviewNote: note })).payout;
      if (kind === "reject")
        return (await walletAdminClient().rejectPayout({ payoutId, reviewNote: note })).payout;
      return (await walletAdminClient().markPayoutPaid({ payoutId, paymentReference: note }))
        .payout;
    },
    onSuccess: (updated, kind) => {
      if (updated) queryClient.setQueryData(walletKeys.admin.payout(payoutId), updated);
      void queryClient.invalidateQueries({ queryKey: walletKeys.admin.queues });
      toast.success(t(`staff.payouts.toast.${kind}`));
      setConfirming(false);
    },
    onError: (failure) => {
      toast.error(errorMessage(failure));
      // Someone else may have decided meanwhile: show what the request is now.
      void queryClient.invalidateQueries({ queryKey: walletKeys.admin.payout(payoutId) });
      void queryClient.invalidateQueries({ queryKey: walletKeys.admin.queues });
    },
  });

  const start = (kind: ReviewAction) => {
    setText("");
    setMissing(false);
    setAction(kind);
    setConfirming(true);
  };
  const confirm = () => {
    if (action === "reject" && !text.trim()) {
      setMissing(true);
      return;
    }
    review.mutate(action);
  };

  if (isPending) return <Skeleton className="h-72" />;
  if (!payout)
    return <EmptyState emoji="🔍" title={error ? errorMessage(error) : t("payout.notFound")} />;

  const format = (value: Date | undefined) =>
    value?.toLocaleString(i18n.language, { dateStyle: "medium", timeStyle: "short" });
  const copyDestination = async () => {
    await navigator.clipboard.writeText(payout.destination);
    toast.success(t("common:actions.copied"));
  };

  return (
    <div>
      <div className="flex items-start justify-between gap-3">
        <UserLine user={payout.user} />
        <PayoutStatusChip status={payout.status} />
      </div>
      <PayoutAmount payout={payout} large className="mt-3" />

      <dl className="mt-3 divide-y-2 divide-line">
        <Fact label={t("payout.form.method")}>{t(payoutMethodKey(payout.method))}</Fact>
        <Fact stack label={t("payout.form.destination")}>
          {payout.destination ? (
            <span className="inline-flex items-start gap-1">
              <span className="whitespace-pre-wrap break-all">{payout.destination}</span>
              <Button
                variant="ghost"
                size="icon-sm"
                className="-my-1.5 shrink-0"
                aria-label={t("staff.payouts.copyDestination")}
                onClick={() => void copyDestination()}
              >
                <Copy className="size-4" />
              </Button>
            </span>
          ) : (
            <span className="text-ink-faint">{t("staff.payouts.destinationErased")}</span>
          )}
        </Fact>
        {payout.note && (
          <Fact stack label={t("staff.payouts.requesterNote")}>
            <span className="whitespace-pre-wrap">{payout.note}</span>
          </Fact>
        )}
        <Fact label={t("staff.payouts.requestedAt")}>{format(toDate(payout.createdAt))}</Fact>
        {payout.reviewedAt && (
          <Fact label={t("staff.payouts.reviewedAt")}>
            {format(toDate(payout.reviewedAt))}
            {payout.reviewer && <span className="text-ink-soft"> · @{payout.reviewer.handle}</span>}
          </Fact>
        )}
        {payout.reviewNote && (
          <Fact stack label={t("staff.payouts.reviewNote")}>
            <span className="whitespace-pre-wrap">{payout.reviewNote}</span>
          </Fact>
        )}
        {payout.paidAt && (
          <Fact label={t("staff.payouts.paidAt")}>{format(toDate(payout.paidAt))}</Fact>
        )}
        {payout.paymentReference && (
          <Fact label={t("payout.paymentReference")}>
            <span className="break-all">{payout.paymentReference}</span>
          </Fact>
        )}
      </dl>

      <div className="mt-4 flex flex-wrap gap-2">
        {payout.status === wallet.PayoutStatus.PENDING && (
          <Button
            variant="lime"
            icon={<Check className="size-4" />}
            onClick={() => start("approve")}
          >
            {t("staff.payouts.approve")}
          </Button>
        )}
        {payout.status === wallet.PayoutStatus.APPROVED && (
          <Button
            variant="lime"
            icon={<BadgeCheck className="size-4" />}
            onClick={() => start("paid")}
          >
            {t("staff.payouts.markPaid")}
          </Button>
        )}
        {(payout.status === wallet.PayoutStatus.PENDING ||
          payout.status === wallet.PayoutStatus.APPROVED) && (
          <Button
            variant="secondary"
            icon={<Ban className="size-4" />}
            onClick={() => start("reject")}
          >
            {t("staff.payouts.reject")}
          </Button>
        )}
        {payout.user && (
          <Button
            variant="ghost"
            icon={<WalletCards className="size-4" />}
            onClick={() => payout.user && onOpenUser(payout.user.id)}
          >
            {t("staff.payouts.openWallet")}
          </Button>
        )}
      </div>

      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t(`staff.payouts.confirm.${action}.title`)}
        description={t(`staff.payouts.confirm.${action}.body`)}
        confirmLabel={t(`staff.payouts.confirm.${action}.confirm`)}
        danger={action === "reject"}
        loading={review.isPending}
        onConfirm={confirm}
      >
        {action === "paid" ? (
          <TextField
            label={t("staff.payouts.paymentReference")}
            hint={t("staff.payouts.paymentReferenceHint")}
            value={text}
            maxLength={PAYMENT_REFERENCE_MAX}
            onChange={(event) => setText(event.target.value)}
            autoComplete="off"
          />
        ) : (
          <TextArea
            label={
              action === "reject" ? t("staff.payouts.rejectReason") : t("staff.payouts.approveNote")
            }
            hint={
              action === "reject"
                ? t("staff.payouts.rejectReasonHint")
                : t("staff.payouts.approveNoteHint")
            }
            error={missing ? t("staff.payouts.rejectReasonRequired") : undefined}
            value={text}
            maxLength={REVIEW_NOTE_MAX}
            onChange={(event) => {
              setText(event.target.value);
              setMissing(false);
            }}
          />
        )}
      </ConfirmDialog>
    </div>
  );
}
