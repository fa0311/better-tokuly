import { toMillis, wallet } from "@tokuly/api";
import { collectedSteps, countdown } from "@tokuly/core/wallet";
import { router } from "expo-router";
import {
  Check,
  Flame,
  Gift,
  History,
  PiggyBank,
  ReceiptText,
  ShoppingBag,
} from "lucide-react-native";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, View } from "react-native";
import { Button } from "#/components/button";
import { EmptyState, Page, Row, Section, Spinner } from "#/components/page";
import { webScreenHref } from "#/features/web/handoff";
import { INK_ON_BRIGHT, sticker, useTheme } from "#/lib/theme";
import { useClaimBonus } from "./actions";
import { useBalance, useDailyBonus, useLedger } from "./data";
import { coinText, EntryRow, EntrySheet } from "./parts";

/** How many of the latest movements the front page shows. */
const RECENT = 5;

/**
 * `/wallet`: the coins and earnings as the server has them, today's login
 * bonus, the latest movements, and the ways onward — the history and the
 * purchases here, buying coins and earnings on the web screens.
 */
export function WalletScreen() {
  const { t } = useTranslation(["wallet", "mobile"]);
  const { colors } = useTheme();
  return (
    <Page testID="wallet-screen">
      <BalanceCard />
      <BonusCard />
      <Recent />
      <Section>
        <Row
          testID="wallet-history"
          icon={<History size={20} color={colors.ink} />}
          label={t("wallet:history.title")}
          onPress={() => router.push("/wallet/history")}
        />
        <Row
          testID="wallet-purchases"
          icon={<ReceiptText size={20} color={colors.ink} />}
          label={t("wallet:home.shortcuts.purchases")}
          hint={t("wallet:home.shortcuts.purchasesHint")}
          onPress={() => router.push("/wallet/purchases")}
        />
      </Section>
      <Section title={t("mobile:wallet.onWeb")}>
        <Row
          testID="wallet-web-buy"
          icon={<ShoppingBag size={20} color={colors.ink} />}
          label={t("wallet:home.buy")}
          hint={t("mobile:wallet.buyHint")}
          onPress={() => router.push(webScreenHref("/wallet/buy") as never)}
        />
        <Row
          testID="wallet-web-earnings"
          icon={<PiggyBank size={20} color={colors.ink} />}
          label={t("wallet:home.shortcuts.earnings")}
          hint={t("wallet:home.shortcuts.earningsHint")}
          onPress={() => router.push(webScreenHref("/wallet/earnings") as never)}
        />
      </Section>
    </Page>
  );
}

/** The balance, front and centre: always the server's number, never one added up here. */
function BalanceCard() {
  const { t } = useTranslation(["wallet", "common"]);
  const { colors } = useTheme();
  const { data: balance, isError, refetch } = useBalance();
  return (
    <View
      testID="wallet-balance-card"
      className="gap-1 rounded-card bg-brand p-5"
      style={sticker(colors)}
      accessibilityLabel={t("home.balance")}
    >
      <Text className="text-sm font-black text-on-brand opacity-80">{t("home.balance")}</Text>
      {balance ? (
        <Text testID="wallet-coins" className="text-5xl font-black text-on-brand">
          {coinText(balance.coins)}
        </Text>
      ) : isError ? (
        <Pressable accessibilityRole="button" onPress={() => void refetch()}>
          <Text className="py-3 font-bold text-on-brand underline">{t("home.balanceError")}</Text>
        </Pressable>
      ) : (
        <View className="h-14 justify-center">
          <Text className="text-3xl font-black text-on-brand opacity-50">🪙 …</Text>
        </View>
      )}
      {balance && balance.earnings > 0n && (
        <Pressable
          testID="wallet-earnings"
          accessibilityRole="button"
          onPress={() => router.push(webScreenHref("/wallet/earnings") as never)}
          className="mt-1 self-start rounded-full bg-white/20 px-3 py-1"
        >
          <Text className="text-sm font-black text-on-brand">
            {t("home.earnings")} {coinText(balance.earnings)}
          </Text>
        </Pressable>
      )}
    </View>
  );
}

/** The clock, a tick every twenty seconds: enough for "次は3時間2分後". */
function useNow(): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 20_000);
    return () => clearInterval(timer);
  }, []);
  return now;
}

/**
 * The daily login bonus: the reward ladder with the streak so far and the
 * claim. "ゲット!" is shown only for coins the server says it granted.
 * Renders nothing on servers where the bonus is switched off.
 */
function BonusCard() {
  const { t } = useTranslation("wallet");
  const { colors } = useTheme();
  const { data: bonus, isPending, refetch } = useDailyBonus();
  /** Coins of the claim made on this screen just now. */
  const [won, setWon] = useState<bigint | null>(null);
  const { claim, claiming } = useClaimBonus(setWon, (message) => Alert.alert(message));
  const now = useNow();
  const unlocksAt = toMillis(bonus?.nextClaimAt);
  const unlocked = !!bonus && !bonus.claimable && unlocksAt > 0 && now >= unlocksAt;

  // Midnight passed with the screen open: today's bonus is there to take.
  // The phone's clock may be ahead of the server's, so it is asked with
  // every tick until it agrees.
  // biome-ignore lint/correctness/useExhaustiveDependencies: `now` is the tick
  useEffect(() => {
    if (unlocked) void refetch();
  }, [unlocked, now, refetch]);

  if (isPending) return <Spinner />;
  if (!bonus || bonus.cycle.length === 0) return null;
  const collected = collectedSteps(bonus);
  const left = countdown(unlocksAt, now);
  const claimLabel = `${t("bonus.claim")} ${coinText(bonus.nextCoins, true)}`;

  return (
    <View
      testID="wallet-bonus"
      className="gap-4 rounded-card bg-surface p-4"
      style={sticker(colors)}
    >
      <View className="flex-row items-center gap-3">
        <View
          className="h-12 w-12 items-center justify-center rounded-2xl bg-sun"
          style={sticker(colors, "sm")}
        >
          <Gift size={24} color={INK_ON_BRIGHT} />
        </View>
        <View className="flex-1">
          <Text className="font-black text-ink">{t("bonus.title")}</Text>
          <Text className="text-sm text-ink-soft">
            {bonus.claimable ? t("bonus.leadClaimable") : t("bonus.leadClaimed")}
          </Text>
        </View>
        {bonus.streak > 0 && (
          <View className="flex-row items-center gap-1 rounded-full bg-coral px-3 py-1">
            <Flame size={14} color={INK_ON_BRIGHT} />
            <Text className="text-sm font-black" style={{ color: INK_ON_BRIGHT }}>
              {t("bonus.streak", { count: bonus.streak })}
            </Text>
          </View>
        )}
      </View>

      <View className="flex-row gap-1" accessibilityLabel={t("bonus.ladder")}>
        {bonus.cycle.map((coins, index) => {
          const day = index + 1;
          const done = day <= collected;
          const next = bonus.claimable && day === bonus.nextCycleDay;
          return (
            <View
              // The ladder is a fixed sequence: the position is the identity.
              key={index}
              className={`flex-1 items-center gap-1 rounded-2xl border-2 py-2 ${
                done
                  ? "border-ink bg-lime"
                  : next
                    ? "border-ink bg-sun"
                    : "border-line bg-surface-2"
              }`}
            >
              <Text
                className={`text-[10px] font-black ${done || next ? "" : "text-ink-soft"}`}
                style={done || next ? { color: INK_ON_BRIGHT } : undefined}
              >
                {t("bonus.day", { day })}
              </Text>
              {done ? (
                <Check size={16} color={INK_ON_BRIGHT} strokeWidth={4} />
              ) : (
                <Text className="text-xs">🪙</Text>
              )}
              <Text
                className={`text-xs font-black ${done || next ? "" : "text-ink-soft"}`}
                style={done || next ? { color: INK_ON_BRIGHT } : undefined}
              >
                {coins.toString()}
              </Text>
            </View>
          );
        })}
      </View>

      {bonus.claimable ? (
        <Button
          testID="wallet-bonus-claim"
          variant="lime"
          size="lg"
          block
          loading={claiming}
          onPress={claim}
        >
          {claimLabel}
        </Button>
      ) : (
        <View
          testID="wallet-bonus-claimed"
          accessibilityRole="summary"
          className="items-center gap-1 rounded-3xl border-2 border-dashed border-line px-4 py-3"
        >
          <Text
            testID={won !== null ? "wallet-bonus-won" : undefined}
            className="text-lg font-black text-ink"
          >
            {won !== null ? `${t("bonus.won")} ${coinText(won, true)}` : t("bonus.claimed")}
          </Text>
          {unlocksAt > 0 && (
            <Text className="text-sm text-ink-soft">
              {left.hours > 0
                ? t("bonus.nextInHours", { hours: left.hours, minutes: left.minutes })
                : t("bonus.nextInMinutes", { minutes: left.minutes })}{" "}
              {coinText(bonus.nextCoins, true)}
            </Text>
          )}
        </View>
      )}

      {bonus.totalClaims > 0 && (
        <Text className="text-center text-xs font-bold text-ink-faint">
          {t("bonus.record", { longest: bonus.longestStreak, total: bonus.totalClaims })}
        </Text>
      )}
    </View>
  );
}

/** The latest movements; they share their data with the history screen. */
function Recent() {
  const { t } = useTranslation("wallet");
  const { entries, isPending } = useLedger(wallet.LedgerAccount.UNSPECIFIED);
  const [open, setOpen] = useState<wallet.LedgerEntry | null>(null);
  return (
    <Section title={t("home.recent")}>
      {isPending ? (
        <Spinner />
      ) : entries.length === 0 ? (
        <EmptyState emoji="🪙" title={t("home.recentEmpty")} body={t("home.recentEmptyBody")} />
      ) : (
        entries
          .slice(0, RECENT)
          .map((entry) => (
            <EntryRow key={entry.id} entry={entry} when="age" onPress={() => setOpen(entry)} />
          ))
      )}
      <EntrySheet entry={open} onClose={() => setOpen(null)} />
    </Section>
  );
}
