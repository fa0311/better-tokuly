import { create } from "@bufbuild/protobuf";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { errorReason, isApiError, live } from "@tokuly/api";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Coins } from "#/components/ui/coins";
import { Dialog } from "#/components/ui/dialog";
import { TextArea, TextField } from "#/components/ui/field";
import { useBalance } from "#/features/wallet";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import { giftClient, liveKeys, useGiftTiers } from "../api";
import { GiftBubble } from "../chat/chat-line";
import { type GiftAttempt, giftAttempt, parseCoins, sameGift, tierFor, tierStyle } from "../model";
import { formatCount } from "../parts";
import { addOwnMessage } from "../room";

/**
 * Sends a gift (super chat): pick an amount, see which of the twelve colours
 * it lands in and how long a message it may carry, look at the preview,
 * confirm, spend. Someone who is short of coins is sent to the wallet.
 */
export function GiftDialog({
  stream,
  open,
  onOpenChange,
}: {
  stream: live.Stream;
  open: boolean;
  onOpenChange(open: boolean): void;
}) {
  const { t, i18n } = useTranslation("live");
  const queryClient = useQueryClient();
  const me = useMe();
  const tiersQuery = useGiftTiers();
  const balance = useBalance();
  const [amountText, setAmountText] = useState("100");
  const [message, setMessage] = useState("");
  const [confirming, setConfirming] = useState(false);
  /**
   * The gift whose answer was lost, if any: confirming exactly that gift again
   * repeats it under its key (nothing is paid twice). Changing the amount or
   * the message, going back or closing makes the next send a new gift.
   */
  const [lost, setLost] = useState<GiftAttempt | null>(null);

  const tiers = tiersQuery.data?.tiers ?? [];
  const minCoins = tiersQuery.data?.minCoins ?? 10n;
  const maxCoins = tiersQuery.data?.maxCoins ?? 100_000n;
  const coins = parseCoins(amountText);
  const inRange = coins !== null && coins >= minCoins && coins <= maxCoins;
  const tier = inRange ? tierFor(tiers, coins) : undefined;
  const maxLength = tier?.maxMessageLength ?? 0;
  const text = message.slice(0, maxLength).trim();
  const short = inRange && balance.coins !== undefined && balance.coins < coins;
  const channelName = stream.summary?.channel?.name ?? "";

  const reset = () => {
    setConfirming(false);
    setMessage("");
    setLost(null);
  };
  const back = () => {
    setConfirming(false);
    setLost(null);
  };
  const send = useMutation({
    mutationFn: async (attempt: GiftAttempt) => ({
      attempt,
      response: await giftClient().sendGift({
        streamId: stream.summary?.id ?? "",
        coins: attempt.coins,
        message: attempt.message,
        idempotencyKey: attempt.key,
      }),
    }),
    onMutate: (attempt) => setLost(attempt),
    onSuccess: ({ attempt, response }) => {
      if (response.message) addOwnMessage(queryClient, response.message);
      void balance.refresh();
      void queryClient.invalidateQueries({ queryKey: liveKeys.gifts });
      void queryClient.invalidateQueries({
        queryKey: liveKeys.supporters(stream.summary?.id ?? ""),
      });
      // Never claim a gift that was not the one sent: an earlier attempt can come back instead.
      if (response.gift && !sameGift(attempt, response.gift))
        toast.warning(
          t("gift.otherSent", {
            coins: formatCount(response.gift.coins, i18n.language),
            name: channelName,
          }),
        );
      else toast.success(t("gift.sent", { name: channelName }));
      reset();
      onOpenChange(false);
    },
    onError: (error) => {
      toast.error(errorMessage(error, { namespace: "live" }));
      // A refusal settles the attempt (nothing was paid); only a lost answer is retried under the same key.
      if (isApiError(error) && errorReason(error)) {
        setLost(null);
        setConfirming(false);
        if (errorReason(error) === "GIFT_RETURNED" || errorReason(error) === "INSUFFICIENT_COINS")
          void balance.refresh();
      }
    },
  });

  const preview = create(live.ChatMessageSchema, {
    kind: live.ChatMessageKind.GIFT,
    author: { user: me?.user },
    text,
    gift: { coins: coins ?? 0n, tier: tier?.level ?? 0 },
  });

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (send.isPending) return;
        if (!next) back();
        onOpenChange(next);
      }}
      title={confirming ? t("gift.confirmTitle") : t("gift.title", { name: channelName })}
      description={confirming ? t("gift.confirmLead") : t("gift.lead")}
      footer={
        confirming ? (
          <>
            <Button variant="ghost" onClick={back} disabled={send.isPending}>
              {t("gift.back")}
            </Button>
            <Button
              loading={send.isPending}
              onClick={() => send.mutate(giftAttempt(lost, coins ?? 0n, text))}
            >
              {t("gift.confirm", { coins: formatCount(coins ?? 0n, i18n.language) })}
            </Button>
          </>
        ) : short ? (
          <Link
            to="/wallet/buy"
            className="pressable sticker inline-flex h-11 items-center rounded-full bg-lime px-5 text-[15px] font-bold text-[#17122b]"
          >
            {t("gift.buyCoins")}
          </Link>
        ) : (
          <Button disabled={!inRange || !tier} onClick={() => setConfirming(true)}>
            {t("gift.next")}
          </Button>
        )
      }
    >
      <div className="flex flex-col gap-4">
        {confirming && lost && !send.isPending && (
          <p className="rounded-2xl bg-sun px-4 py-3 text-sm font-bold text-[#17122b]" role="alert">
            {t("gift.retryHint")}
          </p>
        )}
        {!confirming && (
          <>
            <div className="flex items-center justify-between gap-3 rounded-2xl bg-surface-2 px-4 py-2.5">
              <span className="text-sm font-bold text-ink-soft">{t("gift.balance")}</span>
              {balance.coins === undefined ? (
                <span className="text-sm text-ink-faint">…</span>
              ) : (
                <span data-testid="gift-balance" data-coins={String(balance.coins)}>
                  <Coins amount={balance.coins} />
                </span>
              )}
            </div>
            <fieldset>
              <legend className="mb-2 px-1 text-sm font-bold text-ink-soft">
                {t("gift.tiers")}
              </legend>
              <div className="grid grid-cols-4 gap-1.5 sm:grid-cols-6">
                {tiers.map((option) => (
                  <button
                    key={option.level}
                    type="button"
                    aria-pressed={tier?.level === option.level}
                    aria-label={t("gift.tierLabel", {
                      level: option.level,
                      coins: formatCount(option.minCoins, i18n.language),
                    })}
                    onClick={() => setAmountText(String(option.minCoins))}
                    style={tierStyle(option)}
                    className={cn(
                      "h-10 rounded-xl border-2 text-xs font-black tabular-nums",
                      tier?.level === option.level
                        ? "border-ink shadow-pop-sm"
                        : "border-transparent opacity-80 hover:opacity-100",
                    )}
                  >
                    {formatCount(option.minCoins, i18n.language)}
                  </button>
                ))}
              </div>
            </fieldset>
            <TextField
              label={t("gift.amount")}
              inputMode="numeric"
              value={amountText}
              onChange={(event) => setAmountText(event.target.value)}
              error={
                amountText && !inRange
                  ? t("gift.amountRange", {
                      min: formatCount(minCoins, i18n.language),
                      max: formatCount(maxCoins, i18n.language),
                    })
                  : short
                    ? t("gift.short")
                    : undefined
              }
            />
            {maxLength > 0 ? (
              <TextArea
                label={t("gift.message")}
                hint={t("gift.messageHint", { count: message.length, max: maxLength })}
                value={message}
                maxLength={maxLength}
                onChange={(event) => setMessage(event.target.value)}
              />
            ) : (
              inRange && <p className="px-1 text-sm text-ink-soft">{t("gift.noMessage")}</p>
            )}
          </>
        )}
        {inRange && tier && (
          <div>
            <p className="mb-2 px-1 text-sm font-bold text-ink-soft">{t("gift.preview")}</p>
            <GiftBubble message={preview} tiers={tiers} />
            {tier.highlightSeconds > 0 && (
              <p className="mt-2 px-1 text-xs text-ink-faint">
                {t("gift.highlightFor", { count: Math.round(tier.highlightSeconds / 60) || 1 })}
              </p>
            )}
          </div>
        )}
      </div>
    </Dialog>
  );
}
