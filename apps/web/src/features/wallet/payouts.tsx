import { useMutation, useQueryClient } from "@tanstack/react-query";
import { wallet } from "@tokuly/api";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Coins } from "#/components/ui/coins";
import { Dialog } from "#/components/ui/dialog";
import { TextArea, TextField } from "#/components/ui/field";
import { SelectField } from "#/components/ui/select";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { earningsClient, rememberBalance, walletKeys } from "./api";
import { formatCoins, formatMoney, parseCoins, payoutMoney, payoutProblem } from "./format";
import { StatusChip, type Tone } from "./parts";

const STATUS: Partial<Record<wallet.PayoutStatus, { key: string; tone: Tone }>> = {
  [wallet.PayoutStatus.PENDING]: { key: "pending", tone: "wait" },
  [wallet.PayoutStatus.APPROVED]: { key: "approved", tone: "info" },
  [wallet.PayoutStatus.REJECTED]: { key: "rejected", tone: "bad" },
  [wallet.PayoutStatus.PAID]: { key: "paid", tone: "good" },
  [wallet.PayoutStatus.CANCELLED]: { key: "cancelled", tone: "neutral" },
};

/** i18n key (under `payout.status`) of a payout state. */
export function payoutStatusKey(status: wallet.PayoutStatus): string {
  return `payout.status.${STATUS[status]?.key ?? "unknown"}`;
}

export function PayoutStatusChip({ status }: { status: wallet.PayoutStatus }) {
  const { t } = useTranslation("wallet");
  return (
    <StatusChip tone={STATUS[status]?.tone ?? "neutral"}>{t(payoutStatusKey(status))}</StatusChip>
  );
}

const METHODS = {
  bank: wallet.PayoutMethod.BANK_TRANSFER,
  paypal: wallet.PayoutMethod.PAYPAL,
  giftCard: wallet.PayoutMethod.GIFT_CARD,
  other: wallet.PayoutMethod.OTHER,
} as const;

type MethodKey = keyof typeof METHODS;

/** i18n key (under `payout.method`) of a payout method. */
export function payoutMethodKey(method: wallet.PayoutMethod): string {
  const key = (Object.keys(METHODS) as MethodKey[]).find((name) => METHODS[name] === method);
  return `payout.method.${key ?? "unknown"}`;
}

/** True while a request still holds the user's earnings (waiting for review or for the money). */
export function isOpenPayout(payout: wallet.PayoutRequest): boolean {
  return (
    payout.status === wallet.PayoutStatus.PENDING || payout.status === wallet.PayoutStatus.APPROVED
  );
}

const DESTINATION_MAX = 500;
const NOTE_MAX = 500;

/**
 * The payout request form. Amounts are checked against the server's minimum
 * and the available earnings as they are typed; the server has the last word.
 * Mount it fresh for every request: each mount is one idempotent attempt.
 */
export function PayoutRequestDialog({
  open,
  onOpenChange,
  available,
  terms,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  available: bigint;
  terms: wallet.PayoutTerms;
}) {
  const { t, i18n } = useTranslation(["wallet", "common"]);
  const queryClient = useQueryClient();
  const [idempotencyKey] = useState(() => crypto.randomUUID());
  const [amount, setAmount] = useState(() => available.toString());
  const [method, setMethod] = useState<MethodKey>("bank");
  const [destination, setDestination] = useState("");
  const [note, setNote] = useState("");
  /** Field errors appear once the person has tried to send. */
  const [submitted, setSubmitted] = useState(false);

  const coins = parseCoins(amount);
  const problem = coins === undefined ? "invalid" : payoutProblem(coins, available, terms.minCoins);
  const amountError =
    problem === "invalid"
      ? t("payout.form.amountInvalid")
      : problem === "belowMinimum"
        ? t("payout.form.belowMinimum", { coins: formatCoins(terms.minCoins) })
        : problem === "aboveAvailable"
          ? t("payout.form.aboveAvailable", { coins: formatCoins(available) })
          : undefined;
  const destinationError = destination.trim() ? undefined : t("payout.form.destinationRequired");

  const request = useMutation({
    mutationFn: () =>
      earningsClient().requestPayout({
        coins: coins ?? 0n,
        method: METHODS[method],
        destination: destination.trim(),
        note: note.trim(),
        idempotencyKey,
      }),
    onSuccess: (response) => {
      rememberBalance(queryClient, response.balance);
      void queryClient.invalidateQueries({ queryKey: walletKeys.payouts });
      void queryClient.invalidateQueries({ queryKey: walletKeys.earnings });
      void queryClient.invalidateQueries({ queryKey: walletKeys.ledgers });
      toast.success(t("payout.toast.requested"));
      onOpenChange(false);
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  const submit = () => {
    setSubmitted(true);
    if (!amountError && !destinationError) request.mutate();
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("payout.form.title")}
      description={t("payout.form.lead")}
      footer={
        <>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={request.isPending}>
            {t("common:actions.cancel")}
          </Button>
          <Button loading={request.isPending} onClick={submit}>
            {t("payout.form.submit")}
          </Button>
        </>
      }
    >
      <form
        className="flex flex-col gap-4"
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        <TextField
          label={t("payout.form.amount")}
          inputMode="numeric"
          autoComplete="off"
          value={amount}
          onChange={(event) => setAmount(event.target.value)}
          // The amount is wrong the moment it is typed; do not wait for a submit to say so.
          error={amount.trim() === "" && !submitted ? undefined : amountError}
          hint={t("payout.form.amountHint", {
            available: formatCoins(available),
            minimum: formatCoins(terms.minCoins),
          })}
          trailing={
            <Button variant="ghost" size="sm" onClick={() => setAmount(available.toString())}>
              {t("payout.form.all")}
            </Button>
          }
        />
        <div className="flex items-center justify-between rounded-2xl bg-surface-2 px-4 py-3">
          <span className="text-sm font-bold text-ink-soft">{t("payout.form.youGet")}</span>
          <span className="text-xl font-black">
            {formatMoney(
              payoutMoney(amountError || coins === undefined ? 0n : coins, terms.minorPer100Coins),
              terms.currency,
              i18n.language,
            )}
          </span>
        </div>
        <SelectField<MethodKey>
          label={t("payout.form.method")}
          value={method}
          onChange={setMethod}
          options={(Object.keys(METHODS) as MethodKey[]).map((key) => ({
            value: key,
            label: t(`payout.method.${key}`),
          }))}
        />
        <TextArea
          label={t("payout.form.destination")}
          value={destination}
          maxLength={DESTINATION_MAX}
          onChange={(event) => setDestination(event.target.value)}
          placeholder={t(`payout.form.destinationPlaceholder.${method}`)}
          error={submitted ? destinationError : undefined}
          hint={t("payout.form.destinationHint")}
        />
        <TextArea
          label={t("payout.form.note")}
          value={note}
          maxLength={NOTE_MAX}
          onChange={(event) => setNote(event.target.value)}
          placeholder={t("payout.form.notePlaceholder")}
        />
        {/* Lets the keyboard's "go" key send the form. */}
        <button type="submit" hidden />
      </form>
    </Dialog>
  );
}

/** Withdrawing the user's own pending request; the coins return to their earnings. */
export function useCancelPayout() {
  const { t } = useTranslation("wallet");
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (payoutId: string) => earningsClient().cancelPayoutRequest({ payoutId }),
    onSuccess: (response) => {
      rememberBalance(queryClient, response.balance);
      void queryClient.invalidateQueries({ queryKey: walletKeys.payouts });
      void queryClient.invalidateQueries({ queryKey: walletKeys.earnings });
      void queryClient.invalidateQueries({ queryKey: walletKeys.ledgers });
      toast.success(t("payout.toast.cancelled"));
    },
    onError: (error) => {
      toast.error(errorMessage(error));
      // Most likely reviewed a moment ago: show what it became.
      void queryClient.invalidateQueries({ queryKey: walletKeys.payouts });
    },
  });
}

/** Coins of a payout with the money they turn into. */
export function PayoutAmount({
  payout,
  large,
  className,
}: {
  payout: wallet.PayoutRequest /** For the head of a page or pane. */;
  large?: boolean;
  className?: string;
}) {
  const { i18n } = useTranslation("wallet");
  return (
    <span className={cn("inline-flex flex-wrap items-baseline gap-x-2", className)}>
      <Coins amount={payout.coins} className={cn(large && "text-2xl")} />
      <span className={cn("font-bold text-ink-soft", large ? "text-base" : "text-sm")}>
        → {formatMoney(payout.payoutMinor, payout.currency, i18n.language)}
      </span>
    </span>
  );
}
