import type { quest } from "@tokuly/api";
import { remaining } from "@tokuly/core/quest";
import { useTranslation } from "react-i18next";
import { Text, View } from "react-native";

/** A filled bar for progress towards something, 0–1. It fills the rest of a row. */
export function ProgressBar({
  value,
  label,
  tone = "brand",
  thin,
}: {
  value: number;
  /** What the bar measures, for screen readers. */
  label: string;
  tone?: "brand" | "lime";
  thin?: boolean;
}) {
  const percent = Math.round(Math.min(1, Math.max(0, value)) * 100);
  return (
    <View
      accessibilityRole="progressbar"
      accessibilityLabel={label}
      accessibilityValue={{ min: 0, max: 100, now: percent }}
      className={`flex-1 overflow-hidden rounded-full bg-surface-2 ${thin ? "h-2" : "h-3.5 border-2 border-ink"}`}
    >
      <View
        className={`h-full rounded-full ${tone === "lime" ? "bg-lime" : "bg-brand"}`}
        style={{ width: `${percent}%` }}
      />
    </View>
  );
}

/** The title (称号) someone wears. */
export function TitlePill({ title }: { title: string }) {
  return (
    <View className="rounded-full border-2 border-ink bg-surface px-2 py-0.5">
      <Text className="text-xs font-black text-ink">👑 {title}</Text>
    </View>
  );
}

/** What a mission gives: coins and XP (and a badge, for a few). */
export function RewardLine({ reward }: { reward: quest.Reward | undefined }) {
  const { t } = useTranslation("quest");
  if (!reward) return null;
  return (
    <View className="flex-row flex-wrap items-center gap-2">
      {reward.coins > 0 && <Text className="text-sm font-black text-ink">🪙 {reward.coins}</Text>}
      {reward.xp > 0 && (
        <Text className="text-sm font-black text-brand">{t("xp.plus", { xp: reward.xp })}</Text>
      )}
      {reward.badge && (
        <Text className="text-sm font-black text-ink">{reward.badge.emoji || "🏅"}</Text>
      )}
    </View>
  );
}

/** "あと5時間" until `untilMs`. */
export function useRemainingText(): (untilMs: number, nowMs: number) => string {
  const { t } = useTranslation("quest");
  return (untilMs, nowMs) => {
    const { unit, count } = remaining(untilMs, nowMs);
    return t(`remaining.${unit}`, { count });
  };
}
