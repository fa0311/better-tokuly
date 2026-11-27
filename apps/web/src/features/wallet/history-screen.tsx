import { useInfiniteQuery } from "@tanstack/react-query";
import { wallet } from "@tokuly/api";
import { uniqueById, walletQueries } from "@tokuly/core/wallet";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { errorMessage } from "#/lib/errors";
import { PAGE_SIZE, walletClient } from "./api";
import { BalancePill } from "./balance-pill";
import { LedgerList } from "./ledger";
import { LinkButton } from "./parts";

type Filter = "all" | "coins" | "earnings";

const ACCOUNTS: Record<Filter, wallet.LedgerAccount> = {
  all: wallet.LedgerAccount.UNSPECIFIED,
  coins: wallet.LedgerAccount.COINS,
  earnings: wallet.LedgerAccount.EARNINGS,
};

/** Every movement of the user's coins and earnings, newest first. */
export function HistoryScreen() {
  const { t } = useTranslation("wallet");
  const [filter, setFilter] = useState<Filter>("all");
  const account = ACCOUNTS[filter];
  const query = useInfiniteQuery(walletQueries.ledger(walletClient, account, PAGE_SIZE));
  const entries = uniqueById(query.data?.pages.flatMap((page) => page.entries) ?? []);

  return (
    <Page title={t("history.title")} back="/wallet" actions={<BalancePill />}>
      <Tabs<Filter>
        value={filter}
        onChange={setFilter}
        items={[
          { value: "all", label: t("history.filters.all") },
          { value: "coins", label: t("history.filters.coins") },
          { value: "earnings", label: t("history.filters.earnings") },
        ]}
      />
      {query.isPending ? (
        <div className="mt-4 flex flex-col gap-2">
          {[0, 1, 2, 3, 4].map((row) => (
            <Skeleton key={row} className="h-16" />
          ))}
        </div>
      ) : query.isError ? (
        <EmptyState
          emoji="😵‍💫"
          title={errorMessage(query.error, { namespace: "wallet" })}
          action={
            <Button variant="secondary" onClick={() => void query.refetch()}>
              {t("common:actions.retry")}
            </Button>
          }
        />
      ) : entries.length > 0 ? (
        <>
          <LedgerList entries={entries} grouped />
          <LoadMore query={query} showEnd />
        </>
      ) : filter === "earnings" ? (
        <EmptyState
          emoji="🌱"
          title={t("history.emptyEarnings")}
          body={t("history.emptyEarningsBody")}
        />
      ) : (
        <EmptyState
          emoji="🪙"
          title={t("history.empty")}
          body={t("history.emptyBody")}
          action={
            <LinkButton to="/wallet" variant="lime">
              {t("history.emptyAction")}
            </LinkButton>
          }
        />
      )}
    </Page>
  );
}
