import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toMillis, wallet } from "@tokuly/api";
import {
  formatCoins,
  formatMoney,
  rememberBalance,
  rememberPurchase,
  walletKeys,
} from "@tokuly/core/wallet";
import { ReceiptText } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { CoinIcon, Coins } from "#/components/ui/coins";
import { Dialog } from "#/components/ui/dialog";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { compactDuration } from "#/lib/format";
import { walletClient } from "./api";
import { BalancePill } from "./balance-pill";
import { Fact, LinkButton } from "./parts";
import { DEVELOPMENT_PROVIDER, DevelopmentNote, PurchaseProgress, providerName } from "./purchase";

const BADGES: Partial<Record<wallet.PackageBadge, { key: string; className: string }>> = {
  [wallet.PackageBadge.POPULAR]: { key: "shop.badge.popular", className: "bg-pink text-white" },
  [wallet.PackageBadge.BEST_VALUE]: {
    key: "shop.badge.bestValue",
    className: "bg-lime text-[#17122b]",
  },
  [wallet.PackageBadge.LIMITED]: { key: "shop.badge.limited", className: "bg-sun text-[#17122b]" },
};

/** The coin shop: the packages on sale and the purchase flow. */
export function BuyScreen() {
  const { t } = useTranslation(["wallet", "common"]);
  const { data, isPending, isError, error, refetch } = useQuery({
    queryKey: walletKeys.packages,
    queryFn: () => walletClient().listCoinPackages({}),
  });
  /** The provider picked by the buyer; the server's first one until they choose. */
  const [chosenProvider, setChosenProvider] = useState<string | null>(null);
  const [buying, setBuying] = useState<{ pkg: wallet.CoinPackage; attempt: number } | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);

  const providers = data?.providers ?? [];
  const provider =
    chosenProvider && providers.includes(chosenProvider) ? chosenProvider : (providers[0] ?? "");
  const buy = (pkg: wallet.CoinPackage) => {
    setBuying((previous) => ({ pkg, attempt: (previous?.attempt ?? 0) + 1 }));
    setDialogOpen(true);
  };

  return (
    <Page title={t("shop.title")} back="/wallet" actions={<BalancePill />} wide>
      {isPending ? (
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4">
          {[0, 1, 2, 3, 4, 5].map((card) => (
            <Skeleton key={card} className="h-52 rounded-card" />
          ))}
        </div>
      ) : isError ? (
        <EmptyState
          emoji="😵‍💫"
          title={errorMessage(error, { namespace: "wallet" })}
          action={
            <Button variant="secondary" onClick={() => void refetch()}>
              {t("common:actions.retry")}
            </Button>
          }
        />
      ) : providers.length === 0 ? (
        <EmptyState emoji="🏪" title={t("shop.closed")} body={t("shop.closedBody")} />
      ) : (
        <>
          <p className="px-1 text-sm font-bold text-ink-soft">{t("shop.lead")}</p>
          {providers.length > 1 && (
            <div className="mt-3 flex flex-wrap items-center gap-2">
              <span className="px-1 text-sm font-black text-ink-soft">{t("shop.payWith")}</span>
              <Tabs
                value={provider}
                onChange={setChosenProvider}
                items={providers.map((name) => ({ value: name, label: providerName(name, t) }))}
              />
            </div>
          )}
          {provider === DEVELOPMENT_PROVIDER && <DevelopmentNote className="mt-3" />}
          {data.packages.length > 0 ? (
            <ul className="mt-5 grid grid-cols-2 gap-x-3 gap-y-5 sm:grid-cols-3 lg:grid-cols-4">
              {data.packages.map((pkg) => (
                <li key={pkg.id}>
                  <PackageCard pkg={pkg} onBuy={() => buy(pkg)} />
                </li>
              ))}
            </ul>
          ) : (
            <EmptyState emoji="📦" title={t("shop.empty")} body={t("shop.emptyBody")} />
          )}
        </>
      )}

      <div className="mt-8 flex justify-center">
        <LinkButton
          to="/wallet/purchases"
          variant="ghost"
          className="text-ink-soft"
          icon={<ReceiptText className="size-4" />}
        >
          {t("shop.purchases")}
        </LinkButton>
      </div>

      {buying && (
        <PurchaseDialog
          key={`${buying.pkg.id}-${buying.attempt}`}
          pkg={buying.pkg}
          provider={provider}
          open={dialogOpen}
          onOpenChange={setDialogOpen}
          onRetry={() => buy(buying.pkg)}
        />
      )}
    </Page>
  );
}

/** One package on the shelf; the whole card is the buy button. */
function PackageCard({ pkg, onBuy }: { pkg: wallet.CoinPackage; onBuy(): void }) {
  const { t, i18n } = useTranslation("wallet");
  const badge = BADGES[pkg.badge];
  const endsAt = toMillis(pkg.availableUntil);
  const price = formatMoney(pkg.priceMinor, pkg.currency, i18n.language);

  return (
    <button
      type="button"
      onClick={onBuy}
      aria-label={t("shop.packageLabel", { coins: formatCoins(pkg.coins + pkg.bonusCoins), price })}
      className="pressable sticker relative flex h-full w-full flex-col items-center gap-2 rounded-card bg-surface px-3 pt-6 pb-3 text-center"
    >
      {badge && (
        <span
          className={cn(
            "sticker-sm absolute -top-3 left-1/2 -translate-x-1/2 rounded-full px-3 py-0.5 text-xs font-black whitespace-nowrap",
            badge.className,
          )}
        >
          {t(badge.key)}
        </span>
      )}
      <CoinStack coins={pkg.coins} />
      <span className="text-2xl leading-none font-black tabular-nums">
        {formatCoins(pkg.coins)}
      </span>
      <span
        className={cn(
          "inline-flex h-6 items-center rounded-full px-2.5 text-xs font-black whitespace-nowrap",
          pkg.bonusCoins > 0n ? "bg-lime text-[#17122b]" : "invisible",
        )}
      >
        {t("shop.bonus", { coins: formatCoins(pkg.bonusCoins) })}
      </span>
      {endsAt > 0 && (
        <span className="text-xs font-bold text-ink-soft">
          {t("shop.endsIn", { time: compactDuration(Math.max(0, endsAt - Date.now())) })}
        </span>
      )}
      <span className="sticker-sm mt-auto inline-flex h-10 w-full items-center justify-center rounded-full bg-brand font-black text-on-brand">
        {price}
      </span>
    </button>
  );
}

/** A pile of coins that grows with the size of the package. */
function CoinStack({ coins }: { coins: bigint }) {
  const height =
    coins >= 10_000n ? 5 : coins >= 3_000n ? 4 : coins >= 1_000n ? 3 : coins >= 500n ? 2 : 1;
  return (
    <span aria-hidden className="flex h-[4.5rem] flex-col-reverse items-center justify-start">
      {Array.from({ length: height }, (_, level) => (
        <CoinIcon key={level} className={cn("size-10 text-lg", level > 0 && "-mb-8")} />
      ))}
    </span>
  );
}

/**
 * Buying one package: a confirmation, then the purchase wherever the
 * provider takes it. Mounted fresh for every attempt, which also gives each
 * attempt its own idempotency key (a retry after a lost response returns the
 * same purchase instead of charging twice).
 */
function PurchaseDialog({
  pkg,
  provider,
  open,
  onOpenChange,
  onRetry,
}: {
  pkg: wallet.CoinPackage;
  provider: string;
  open: boolean;
  onOpenChange(open: boolean): void;
  onRetry(): void;
}) {
  const { t, i18n } = useTranslation(["wallet", "common"]);
  const queryClient = useQueryClient();
  const [idempotencyKey] = useState(() => crypto.randomUUID());
  const [purchaseId, setPurchaseId] = useState<string | null>(null);
  const purchase = useQuery({
    queryKey: walletKeys.purchase(purchaseId ?? ""),
    queryFn: async () =>
      (await walletClient().getPurchase({ purchaseId: purchaseId ?? "" })).purchase ?? null,
    enabled: purchaseId !== null,
  }).data;

  const start = useMutation({
    mutationFn: () => walletClient().startPurchase({ packageId: pkg.id, provider, idempotencyKey }),
    onSuccess: (response) => {
      rememberBalance(queryClient, response.balance);
      rememberPurchase(queryClient, response.purchase);
      if (response.purchase) setPurchaseId(response.purchase.id);
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "wallet" })),
  });

  const development = provider === DEVELOPMENT_PROVIDER;
  const price = formatMoney(pkg.priceMinor, pkg.currency, i18n.language);
  const settled = purchase && purchase.status !== wallet.PurchaseStatus.PENDING;

  if (purchase) {
    return (
      <Dialog open={open} onOpenChange={onOpenChange} title={t("shop.dialog.title")}>
        <PurchaseProgress
          purchase={purchase}
          celebrate
          footer={
            purchase.status === wallet.PurchaseStatus.COMPLETED ? (
              <Button block onClick={() => onOpenChange(false)}>
                {t("common:actions.done")}
              </Button>
            ) : settled ? (
              <Button block onClick={onRetry}>
                {t("purchase.actions.retry")}
              </Button>
            ) : null
          }
        />
      </Dialog>
    );
  }

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("shop.dialog.title")}
      description={t("shop.dialog.lead")}
      footer={
        <>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={start.isPending}>
            {t("common:actions.cancel")}
          </Button>
          <Button loading={start.isPending} onClick={() => start.mutate()}>
            {development
              ? t("shop.dialog.confirmDevelopment")
              : t("shop.dialog.confirm", { price })}
          </Button>
        </>
      }
    >
      <dl className="divide-y-2 divide-line">
        <Fact label={t("shop.dialog.coins")}>
          <Coins amount={pkg.coins} />
        </Fact>
        {pkg.bonusCoins > 0n && (
          <Fact label={t("shop.dialog.bonus")}>
            <Coins amount={pkg.bonusCoins} signed />
          </Fact>
        )}
        <Fact label={t("shop.dialog.total")}>
          <Coins amount={pkg.coins + pkg.bonusCoins} className="text-xl" />
        </Fact>
        <Fact label={t("shop.dialog.price")}>
          <span className={cn(development && "text-ink-soft line-through")}>{price}</span>
          {development && <span className="ml-2">{t("shop.dialog.free")}</span>}
        </Fact>
        <Fact label={t("shop.dialog.provider")}>{providerName(provider, t)}</Fact>
      </dl>
      {development && <DevelopmentNote className="mt-3" />}
    </Dialog>
  );
}
