import { toDate, wallet } from "@tokuly/api";
import { daysAgo, groupByDay } from "@tokuly/core/wallet";
import { router } from "expo-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { FlatList, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Button } from "#/components/button";
import { EmptyState, ErrorNote, Spinner } from "#/components/page";
import { Tabs } from "#/components/tabs";
import { errorMessage } from "#/lib/errors";
import { useLedger } from "./data";
import { EntryRow, EntrySheet } from "./parts";

type Filter = "all" | "coins" | "earnings";

const ACCOUNTS: Record<Filter, wallet.LedgerAccount> = {
  all: wallet.LedgerAccount.UNSPECIFIED,
  coins: wallet.LedgerAccount.COINS,
  earnings: wallet.LedgerAccount.EARNINGS,
};

/** `/wallet/history`: every movement of the coins and earnings, newest first, by day. */
export function HistoryScreen() {
  const { t, i18n } = useTranslation("wallet");
  const [filter, setFilter] = useState<Filter>("all");
  const ledger = useLedger(ACCOUNTS[filter]);
  const [open, setOpen] = useState<wallet.LedgerEntry | null>(null);
  const now = new Date();
  const days = groupByDay(ledger.entries, (entry) => toDate(entry.createdAt));

  const dayLabel = (date: Date | undefined) => {
    if (!date) return "";
    const ago = daysAgo(date, now);
    if (ago === 0) return t("ledger.today");
    if (ago === 1) return t("ledger.yesterday");
    return date.toLocaleDateString(i18n.language, {
      month: "long",
      day: "numeric",
      weekday: "short",
    });
  };

  return (
    <SafeAreaView edges={["bottom"]} className="flex-1 bg-bg">
      <View className="px-4 pt-4 pb-2">
        <Tabs
          testID="wallet-history-filter"
          value={filter}
          onChange={setFilter}
          items={(["all", "coins", "earnings"] as const).map((value) => ({
            value,
            label: t(`history.filters.${value}`),
          }))}
        />
      </View>
      <FlatList
        testID="wallet-history-list"
        data={days}
        keyExtractor={(day) => day.day}
        contentContainerClassName="gap-4 px-4 pt-2 pb-12"
        onEndReached={() => {
          if (ledger.hasNextPage && !ledger.isFetchingNextPage) void ledger.fetchNextPage();
        }}
        renderItem={({ item: day }) => (
          <View className="gap-2">
            <Text testID="wallet-history-day" className="px-1 text-xs font-black text-ink-soft">
              {dayLabel(day.date)}
            </Text>
            <View className="overflow-hidden rounded-card bg-surface">
              {day.items.map((entry) => (
                <EntryRow key={entry.id} entry={entry} when="time" onPress={() => setOpen(entry)} />
              ))}
            </View>
          </View>
        )}
        ListEmptyComponent={
          ledger.isPending ? (
            <Spinner />
          ) : ledger.isError ? (
            <ErrorNote>{errorMessage(ledger.error, { namespace: "wallet" })}</ErrorNote>
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
                <Button variant="secondary" onPress={() => router.back()}>
                  {t("history.emptyAction")}
                </Button>
              }
            />
          )
        }
        ListFooterComponent={ledger.isFetchingNextPage ? <Spinner /> : undefined}
      />
      <EntrySheet entry={open} onClose={() => setOpen(null)} />
    </SafeAreaView>
  );
}
