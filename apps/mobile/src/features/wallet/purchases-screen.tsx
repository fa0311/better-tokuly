import { toDate, wallet } from "@tokuly/api";
import { formatMoney } from "@tokuly/core/wallet";
import { router } from "expo-router";
import { useTranslation } from "react-i18next";
import { FlatList, Pressable, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Button } from "#/components/button";
import { EmptyState, ErrorNote, Spinner } from "#/components/page";
import { webScreenHref } from "#/features/web/handoff";
import { errorMessage } from "#/lib/errors";
import { useTheme } from "#/lib/theme";
import { usePurchases } from "./data";
import { coinText } from "./parts";

/** `wallet:` key of a purchase's state (its last segment names the state in test IDs). */
function statusKey(status: wallet.PurchaseStatus): string {
  switch (status) {
    case wallet.PurchaseStatus.COMPLETED:
      return "purchase.status.completed";
    case wallet.PurchaseStatus.FAILED:
      return "purchase.status.failed";
    case wallet.PurchaseStatus.CANCELLED:
      return "purchase.status.cancelled";
    case wallet.PurchaseStatus.EXPIRED:
      return "purchase.status.expired";
    default:
      return "purchase.status.pending";
  }
}

/**
 * `/wallet/purchases`: every attempt to buy coins, newest first, with its
 * state as the server has it. Paying, checking and cancelling one happen on
 * its web screen.
 */
export function PurchasesScreen() {
  const { t, i18n } = useTranslation(["wallet", "mobile"]);
  const { colors } = useTheme();
  const list = usePurchases();
  return (
    <SafeAreaView edges={["bottom"]} className="flex-1 bg-bg">
      <FlatList
        testID="wallet-purchases-list"
        data={list.purchases}
        keyExtractor={(purchase) => purchase.id}
        contentContainerClassName="px-4 pt-4 pb-12"
        onEndReached={() => {
          if (list.hasNextPage && !list.isFetchingNextPage) void list.fetchNextPage();
        }}
        renderItem={({ item: purchase, index }) => {
          const pending = purchase.status === wallet.PurchaseStatus.PENDING;
          const done = purchase.status === wallet.PurchaseStatus.COMPLETED;
          return (
            <Pressable
              testID={`wallet-purchase-${purchase.id}`}
              accessibilityRole="button"
              onPress={() =>
                router.push(webScreenHref(`/wallet/purchases/${purchase.id}`) as never)
              }
              className={`min-h-16 flex-row items-center gap-3 border-b border-line bg-surface px-4 py-3 active:bg-surface-2 ${
                index === 0 ? "rounded-t-card" : ""
              } ${index === list.purchases.length - 1 ? "rounded-b-card" : ""}`}
            >
              <View className="flex-1 gap-0.5">
                <Text className="font-bold text-ink">
                  {coinText(purchase.coins + purchase.bonusCoins)}
                </Text>
                <Text className="text-sm text-ink-soft">
                  {formatMoney(purchase.priceMinor, purchase.currency, i18n.language)} ·{" "}
                  {toDate(purchase.createdAt)?.toLocaleDateString(i18n.language)}
                </Text>
              </View>
              <Text
                testID={`wallet-purchase-${statusKey(purchase.status).split(".").pop()}`}
                className="overflow-hidden rounded-full px-2.5 py-1 text-xs font-black"
                style={{
                  backgroundColor: done ? colors.lime : pending ? colors.sun : colors["surface-2"],
                  color: done || pending ? "#17122b" : colors["ink-soft"],
                }}
              >
                {t(`wallet:${statusKey(purchase.status)}`)}
              </Text>
            </Pressable>
          );
        }}
        ListEmptyComponent={
          list.isPending ? (
            <Spinner />
          ) : list.isError ? (
            <ErrorNote>{errorMessage(list.error, { namespace: "wallet" })}</ErrorNote>
          ) : (
            <EmptyState
              emoji="🧾"
              title={t("wallet:purchases.empty")}
              body={t("wallet:purchases.emptyBody")}
              action={
                <Button
                  variant="secondary"
                  onPress={() => router.push(webScreenHref("/wallet/buy") as never)}
                >
                  {t("wallet:purchases.emptyAction")}
                </Button>
              }
            />
          )
        }
        ListHeaderComponent={
          <Text className="px-1 pb-2 text-xs text-ink-soft">
            {t("mobile:wallet.purchasesLead")}
          </Text>
        }
        ListFooterComponent={list.isFetchingNextPage ? <Spinner /> : undefined}
      />
    </SafeAreaView>
  );
}
