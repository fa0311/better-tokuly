import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toMillis, wallet } from "@tokuly/api";
import { countdown, rememberBalance, rememberPurchase } from "@tokuly/core/wallet";
import { CircleCheck, CircleX, Clock3, ExternalLink, FlaskConical, Hourglass } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Coins } from "#/components/ui/coins";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { walletClient } from "./api";
import { CoinBurst, useCoinBurst } from "./celebration";
import { type CheckoutStep, checkoutStep, safeCheckoutUrl } from "./format";
import { StatusChip, type Tone } from "./parts";

/** Name of the built-in provider that hands out coins without taking money. */
export const DEVELOPMENT_PROVIDER = "development";

/** A provider's name for people; the development provider says what it is. */
export function providerName(provider: string, t: (key: string) => string): string {
  if (provider === DEVELOPMENT_PROVIDER) return t("provider.development");
  return provider ? provider.charAt(0).toUpperCase() + provider.slice(1) : t("provider.unknown");
}

const STEP_TONES: Record<CheckoutStep, Tone> = {
  completed: "good",
  redirect: "wait",
  waiting: "wait",
  failed: "bad",
  cancelled: "neutral",
  expired: "neutral",
};

export function PurchaseStatusChip({ purchase }: { purchase: wallet.Purchase }) {
  const { t } = useTranslation("wallet");
  const step = checkoutStep(purchase);
  return (
    <StatusChip tone={STEP_TONES[step]}>
      {t(`purchase.status.${step === "redirect" || step === "waiting" ? "pending" : step}`)}
    </StatusChip>
  );
}

/** Marks a purchase made with the development provider: no money moved. */
export function DevelopmentNote({ className }: { className?: string }) {
  const { t } = useTranslation("wallet");
  return (
    <p
      className={cn(
        "flex items-start gap-2 rounded-2xl border-2 border-dashed border-ink-faint bg-surface-2 px-3 py-2.5 text-sm font-bold text-ink-soft",
        className,
      )}
    >
      <FlaskConical className="mt-0.5 size-4 shrink-0" />
      {t("provider.developmentNote")}
    </p>
  );
}

/** Confirming and cancelling a purchase, with the results written to the cache. */
export function usePurchaseActions(purchaseId: string) {
  const { t } = useTranslation("wallet");
  const queryClient = useQueryClient();
  const confirm = useMutation({
    mutationFn: (providerProof: string) =>
      walletClient().confirmPurchase({ purchaseId, providerProof }),
    onSuccess: (response) => {
      rememberPurchase(queryClient, response.purchase);
      rememberBalance(queryClient, response.balance);
    },
    onError: (error) => toast.error(errorMessage(error)),
  });
  const cancel = useMutation({
    mutationFn: () => walletClient().cancelPurchase({ purchaseId }),
    onSuccess: (response) => {
      rememberPurchase(queryClient, response.purchase);
      toast(t("purchase.toast.cancelled"));
    },
    onError: (error) => toast.error(errorMessage(error)),
  });
  /** Asks the provider on the user's request and says so when the money has not arrived. */
  const check = (providerProof = "") =>
    confirm.mutate(providerProof, {
      onSuccess: (response) => {
        if (response.purchase?.status === wallet.PurchaseStatus.PENDING)
          toast(t("purchase.toast.stillPending"));
      },
    });
  return { confirm, cancel, check };
}

/**
 * Where a purchase stands and what the buyer can do next: pay at the
 * provider, confirm, cancel, or enjoy the coins. Shown in the shop's
 * purchase dialog and on the purchase's own page.
 */
export function PurchaseProgress({
  purchase,
  celebrate,
  footer,
}: {
  purchase: wallet.Purchase /** Throw coins when the purchase is (or becomes) completed. */;
  celebrate?: boolean /** Navigation under the state (close, back to the shop…). */;
  footer?: ReactNode;
}) {
  const { t } = useTranslation("wallet");
  const step = checkoutStep(purchase);
  const { confirm, cancel, check } = usePurchaseActions(purchase.id);
  const { burst, fire } = useCoinBurst();

  useEffect(() => {
    if (celebrate && step === "completed") fire();
    // `fire` is a fresh function every render; the burst belongs to the moment the purchase completes.
  }, [celebrate, step]);

  // Coming back from the provider's page: ask right away whether the payment arrived.
  const pending = step === "redirect" || step === "waiting";
  const askProvider = confirm.mutate;
  useEffect(() => {
    if (!pending) return;
    const onVisible = () => {
      if (document.visibilityState === "visible") askProvider("");
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => document.removeEventListener("visibilitychange", onVisible);
  }, [pending, askProvider]);

  const total = purchase.coins + purchase.bonusCoins;
  return (
    <div className="flex flex-col items-center gap-3 text-center">
      {step === "completed" && (
        <>
          <span className="relative">
            <span className="sticker animate-pop-in grid size-20 place-items-center rounded-full bg-lime text-[#17122b]">
              <CircleCheck className="size-10" />
            </span>
            <CoinBurst burst={burst} />
          </span>
          <p className="text-xl font-black">{t("purchase.completed.title")}</p>
          <Coins amount={total} signed size="lg" />
          {purchase.provider === DEVELOPMENT_PROVIDER && <DevelopmentNote className="text-left" />}
        </>
      )}

      {pending && (
        <>
          <span className="sticker grid size-20 place-items-center rounded-full bg-sun text-[#17122b]">
            <Hourglass className="size-9" />
          </span>
          <p className="text-xl font-black">
            {t(step === "redirect" ? "purchase.redirect.title" : "purchase.waiting.title")}
          </p>
          <p className="max-w-sm text-sm leading-relaxed text-ink-soft">
            {t(step === "redirect" ? "purchase.redirect.body" : "purchase.waiting.body", {
              provider: providerName(purchase.provider, t),
            })}
          </p>
          <ExpiryNote purchase={purchase} />
          <ProviderCheckout purchase={purchase} />
          <div className="flex flex-wrap justify-center gap-2">
            <Button variant="secondary" loading={confirm.isPending} onClick={() => check()}>
              {t("purchase.actions.check")}
            </Button>
            <Button variant="ghost" loading={cancel.isPending} onClick={() => cancel.mutate()}>
              {t("purchase.actions.cancel")}
            </Button>
          </div>
        </>
      )}

      {(step === "failed" || step === "cancelled" || step === "expired") && (
        <>
          <span className="sticker grid size-20 place-items-center rounded-full bg-surface-3 text-ink">
            {step === "expired" ? <Clock3 className="size-9" /> : <CircleX className="size-9" />}
          </span>
          <p className="text-xl font-black">{t(`purchase.${step}.title`)}</p>
          <p className="max-w-sm text-sm leading-relaxed text-ink-soft">
            {t(`purchase.${step}.body`)}
          </p>
          {step === "failed" && purchase.failureReason && (
            <p className="rounded-full bg-surface-2 px-3 py-1 font-mono text-xs font-bold text-ink-soft">
              {purchase.failureReason}
            </p>
          )}
          {/* A payment always wins: if money did leave the account, asking again turns this purchase into coins. */}
          <Button variant="ghost" size="sm" loading={confirm.isPending} onClick={() => check()}>
            {t("purchase.actions.paidAnyway")}
          </Button>
        </>
      )}

      {footer && <div className="mt-2 flex w-full flex-wrap justify-center gap-2">{footer}</div>}
    </div>
  );
}

/**
 * The provider's own part of an unpaid purchase.
 *
 * A provider with a hosted page gives a `checkout_url`: the buyer pays there
 * in another tab and this screen picks the result up when they return (or as
 * soon as the provider's webhook pushes `purchase_updated`). A provider with
 * an embedded checkout (a card form, a wallet button) gets a case here that
 * renders its form and hands the SDK's proof of payment to
 * `usePurchaseActions(purchase.id).check(proof)`.
 */
function ProviderCheckout({ purchase }: { purchase: wallet.Purchase }) {
  const { t } = useTranslation("wallet");
  const url = safeCheckoutUrl(purchase.checkoutUrl);
  if (!url) return null;
  return (
    <a
      href={url}
      target="_blank"
      rel="noopener noreferrer"
      className="pressable sticker inline-flex h-14 items-center justify-center gap-2 rounded-full bg-brand px-7 text-lg font-bold text-on-brand"
    >
      {t("purchase.actions.openCheckout")}
      <ExternalLink className="size-5" />
    </a>
  );
}

/** How long an unpaid purchase can still be paid. */
function ExpiryNote({ purchase }: { purchase: wallet.Purchase }) {
  const { t } = useTranslation("wallet");
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(timer);
  }, []);
  const expiresAt = toMillis(purchase.expiresAt);
  if (!expiresAt || expiresAt <= now) return null;
  const left = countdown(expiresAt, now);
  return (
    <p className="text-sm font-bold text-ink-soft">
      {left.hours > 0
        ? t("purchase.expiresInHours", { hours: left.hours, minutes: left.minutes })
        : t("purchase.expiresInMinutes", { minutes: left.minutes })}
    </p>
  );
}
