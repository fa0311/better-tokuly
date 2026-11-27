import type { wallet } from "@tokuly/api";
import {
  formatMoney,
  limitPresets,
  limitSummary,
  ownLimitProblem,
  resetDay,
  usedShare,
} from "@tokuly/core/wallet";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Text, View } from "react-native";
import { Button } from "#/components/button";
import { EmptyState, Page, Section, Spinner } from "#/components/page";
import { TextField } from "#/components/text-field";
import { errorMessage, reasonText } from "#/lib/errors";
import { sticker, useTheme } from "#/lib/theme";
import { useSetSpendingLimit } from "./actions";
import { useSpendingLimit } from "./data";

const yen = (amount: bigint, language: string) => formatMoney(amount, "JPY", language);

/**
 * `/wallet/limit`: this month's spending against the monthly limit, and
 * choosing one's own. Under 18 only up to the ceiling; the server decides.
 */
export function SpendingLimitScreen() {
  const { t } = useTranslation(["wallet", "common"]);
  const { data: limit, isPending, error, refetch } = useSpendingLimit();
  return (
    <Page testID="wallet-limit-screen">
      {isPending ? (
        <Spinner />
      ) : !limit ? (
        <EmptyState
          emoji="😵‍💫"
          title={errorMessage(error, { namespace: "wallet" })}
          action={
            <Button variant="secondary" onPress={() => void refetch()}>
              {t("common:actions.retry")}
            </Button>
          }
        />
      ) : (
        <>
          <ThisMonth limit={limit} />
          <ChooseLimit limit={limit} />
        </>
      )}
    </Page>
  );
}

function ThisMonth({ limit }: { limit: wallet.SpendingLimit }) {
  const { t, i18n } = useTranslation("wallet");
  const { colors } = useTheme();
  const share = usedShare(limit);
  const line = limitSummary(limit, i18n.language);
  return (
    <View className="gap-3 rounded-card bg-surface p-5" style={sticker(colors)}>
      <Text testID="wallet-limit-summary" className="text-lg font-black text-ink">
        {t(line.key, line.values)}
      </Text>
      <View className="flex-row items-baseline justify-between">
        <Text className="text-sm font-bold text-ink-soft">{t("limit.spent")}</Text>
        <Text className="text-2xl font-black text-ink">{yen(limit.spentYen, i18n.language)}</Text>
      </View>
      {share !== null && limit.limitYen !== undefined && (
        <>
          <View
            accessibilityRole="progressbar"
            accessibilityValue={{ min: 0, max: 100, now: Math.round(share * 100) }}
            className="h-4 overflow-hidden rounded-full border-2 border-ink bg-surface-2"
          >
            <View
              className={`h-full ${share >= 1 ? "bg-coral" : share >= 0.8 ? "bg-sun" : "bg-lime"}`}
              style={{ width: `${share * 100}%` }}
            />
          </View>
          <View className="flex-row justify-between">
            <Text className="text-xs font-bold text-ink-soft">
              {t("limit.of", { amount: yen(limit.limitYen, i18n.language) })}
            </Text>
            <Text className="text-xs font-bold text-ink-soft">
              {t("limit.resets", { date: resetDay(limit, i18n.language) })}
            </Text>
          </View>
        </>
      )}
      {limit.ceilingYen !== undefined && (
        <Text className="rounded-2xl bg-surface-2 p-3 text-sm font-bold text-ink">
          {t("limit.ceiling", { amount: yen(limit.ceilingYen, i18n.language) })}
        </Text>
      )}
      <Text className="text-xs text-ink-faint">{t("limit.counts")}</Text>
    </View>
  );
}

/** One-tap amounts, any amount typed in, and (from 18) no limit at all. */
function ChooseLimit({ limit }: { limit: wallet.SpendingLimit }) {
  const { t, i18n } = useTranslation("wallet");
  const [typed, setTyped] = useState("");
  const save = useSetSpendingLimit(
    (limitYen) => {
      setTyped("");
      Alert.alert(
        limitYen === null
          ? t("limit.cleared")
          : t("limit.saved", { amount: yen(limitYen, i18n.language) }),
      );
    },
    (message) => Alert.alert(message),
  );
  const typedYen = /^\d{1,9}$/.test(typed) ? BigInt(typed) : null;
  const problem =
    typed === ""
      ? null
      : typedYen === null
        ? "SPENDING_LIMIT_INVALID"
        : ownLimitProblem(limit, typedYen);
  const canClear = limit.ownLimitYen !== undefined && ownLimitProblem(limit, null) === null;

  return (
    <Section title={t("limit.choose")}>
      <View className="gap-4 p-4">
        <Text testID="wallet-limit-own" className="text-sm font-bold text-ink">
          {t("limit.own")}:{" "}
          {limit.ownLimitYen === undefined
            ? t("limit.unlimited")
            : yen(limit.ownLimitYen, i18n.language)}
        </Text>
        <View className="flex-row flex-wrap gap-2">
          {limitPresets(limit).map((preset) => (
            <Button
              key={preset.toString()}
              testID={`wallet-limit-preset-${preset}`}
              variant={limit.ownLimitYen === preset ? "primary" : "secondary"}
              size="sm"
              disabled={save.isPending}
              onPress={() => save.mutate(preset)}
            >
              {yen(preset, i18n.language)}
            </Button>
          ))}
        </View>
        <TextField
          testID="wallet-limit-input"
          label={t("limit.custom")}
          keyboardType="number-pad"
          value={typed}
          onChangeText={(text) => setTyped(text.replace(/[^\d]/g, ""))}
          error={problem ? reasonText(problem, { namespace: "wallet" }) : undefined}
        />
        <Button
          testID="wallet-limit-save"
          block
          disabled={typedYen === null || problem !== null}
          loading={save.isPending}
          onPress={() => typedYen !== null && save.mutate(typedYen)}
        >
          {t("limit.save")}
        </Button>
        {canClear && (
          <Button variant="ghost" disabled={save.isPending} onPress={() => save.mutate(null)}>
            {t("limit.clear")}
          </Button>
        )}
      </View>
    </Section>
  );
}
