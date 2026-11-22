import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { toDate, wallet } from "@tokuly/api";
import { ChevronRight, ShoppingBag } from "lucide-react";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { Card } from "#/components/ui/card";
import { Coins } from "#/components/ui/coins";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { PAGE_SIZE, walletClient, walletKeys } from "./api";
import { formatMoney } from "./format";
import { uniqueById } from "./ledger";
import { Fact, LinkButton } from "./parts";
import {
  DEVELOPMENT_PROVIDER,
  PurchaseProgress,
  PurchaseStatusChip,
  providerName,
  usePurchaseActions,
} from "./purchase";

/** Every attempt to buy coins, newest first. */
export function PurchasesScreen() {
  const { t, i18n } = useTranslation(["wallet", "common"]);
  const query = useInfiniteQuery({
    queryKey: walletKeys.purchases,
    queryFn: ({ pageParam }) =>
      walletClient().listPurchases({ page: { limit: PAGE_SIZE, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const purchases = uniqueById(query.data?.pages.flatMap((page) => page.purchases) ?? []);

  return (
    <Page title={t("purchases.title")} back="/wallet/buy">
      {query.isPending ? (
        <div className="flex flex-col gap-2">
          {[0, 1, 2].map((row) => (
            <Skeleton key={row} className="h-16" />
          ))}
        </div>
      ) : query.isError ? (
        <EmptyState
          emoji="😵‍💫"
          title={errorMessage(query.error)}
          action={
            <Button variant="secondary" onClick={() => void query.refetch()}>
              {t("common:actions.retry")}
            </Button>
          }
        />
      ) : purchases.length > 0 ? (
        <>
          <ul>
            {purchases.map((purchase) => (
              <li key={purchase.id}>
                <Link
                  to="/wallet/purchases/$purchaseId"
                  params={{ purchaseId: purchase.id }}
                  className="flex items-center gap-3 rounded-2xl px-3 py-2.5 transition-colors hover:bg-surface-2"
                >
                  <span className="grid size-11 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-lime text-[#17122b]">
                    <ShoppingBag className="size-5" />
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="flex items-center gap-2">
                      <Coins amount={purchase.coins + purchase.bonusCoins} />
                      <PurchaseStatusChip purchase={purchase} />
                    </span>
                    <span className="block truncate text-sm text-ink-soft">
                      {toDate(purchase.createdAt)?.toLocaleString(i18n.language, {
                        dateStyle: "medium",
                        timeStyle: "short",
                      })}
                      {" · "}
                      {purchase.provider === DEVELOPMENT_PROVIDER
                        ? providerName(purchase.provider, t)
                        : formatMoney(purchase.priceMinor, purchase.currency, i18n.language)}
                    </span>
                  </span>
                  <ChevronRight className="size-5 shrink-0 text-ink-faint" />
                </Link>
              </li>
            ))}
          </ul>
          <LoadMore query={query} showEnd />
        </>
      ) : (
        <EmptyState
          emoji="🧾"
          title={t("purchases.empty")}
          body={t("purchases.emptyBody")}
          action={
            <LinkButton to="/wallet/buy" variant="lime">
              {t("purchases.emptyAction")}
            </LinkButton>
          }
        />
      )}
    </Page>
  );
}

/**
 * One purchase: its state, what to do next, and the receipt. This is also
 * where a provider's hosted checkout sends the buyer back to (optionally
 * with `?proof=…`, whatever the provider hands over as proof of payment), so
 * an unpaid purchase is checked with the provider as soon as the page opens.
 */
export function PurchaseScreen({ purchaseId, proof }: { purchaseId: string; proof?: string }) {
  const { t, i18n } = useTranslation("wallet");
  const { data: purchase, isPending } = useQuery({
    queryKey: walletKeys.purchase(purchaseId),
    queryFn: async () => (await walletClient().getPurchase({ purchaseId })).purchase ?? null,
  });
  const { confirm } = usePurchaseActions(purchaseId);

  const unpaid = purchase?.status === wallet.PurchaseStatus.PENDING;
  const askProvider = confirm.mutate;
  const asked = useRef(false);
  useEffect(() => {
    if (!unpaid || asked.current) return;
    asked.current = true;
    askProvider(proof ?? "");
  }, [unpaid, proof, askProvider]);

  const format = (value: Date | undefined) =>
    value?.toLocaleString(i18n.language, { dateStyle: "medium", timeStyle: "short" });
  return (
    <Page title={t("purchases.detailTitle")} back="/wallet/purchases">
      {isPending ? (
        <Skeleton className="h-80 rounded-card" />
      ) : !purchase ? (
        <EmptyState
          emoji="🧾"
          title={t("purchases.notFound")}
          action={
            <LinkButton to="/wallet/purchases" variant="secondary">
              {t("purchases.title")}
            </LinkButton>
          }
        />
      ) : (
        <div className="flex flex-col gap-5">
          <Card className="py-8">
            <PurchaseProgress
              purchase={purchase}
              footer={
                purchase.status === wallet.PurchaseStatus.COMPLETED ? (
                  <LinkButton to="/wallet" variant="secondary">
                    {t("purchases.toWallet")}
                  </LinkButton>
                ) : purchase.status === wallet.PurchaseStatus.PENDING ? null : (
                  <LinkButton to="/wallet/buy">{t("purchases.toShop")}</LinkButton>
                )
              }
            />
          </Card>
          <Card className="py-2">
            <dl className="divide-y-2 divide-line">
              <Fact label={t("purchases.receipt.status")}>
                <PurchaseStatusChip purchase={purchase} />
              </Fact>
              <Fact label={t("purchases.receipt.coins")}>
                <Coins amount={purchase.coins} />
              </Fact>
              {purchase.bonusCoins > 0n && (
                <Fact label={t("purchases.receipt.bonus")}>
                  <Coins amount={purchase.bonusCoins} signed />
                </Fact>
              )}
              <Fact label={t("purchases.receipt.price")}>
                {formatMoney(purchase.priceMinor, purchase.currency, i18n.language)}
                {purchase.provider === DEVELOPMENT_PROVIDER && (
                  <span className="ml-2 text-sm text-ink-soft">
                    {t("purchases.receipt.notCharged")}
                  </span>
                )}
              </Fact>
              <Fact label={t("purchases.receipt.provider")}>
                {providerName(purchase.provider, t)}
              </Fact>
              <Fact label={t("purchases.receipt.created")}>
                {format(toDate(purchase.createdAt))}
              </Fact>
              {purchase.completedAt && (
                <Fact label={t("purchases.receipt.completed")}>
                  {format(toDate(purchase.completedAt))}
                </Fact>
              )}
              <Fact stack label={t("purchases.receipt.id")}>
                <span className="break-all font-mono text-xs font-medium text-ink-soft">
                  {purchase.id}
                </span>
              </Fact>
            </dl>
          </Card>
        </div>
      )}
    </Page>
  );
}
