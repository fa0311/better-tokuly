import type { quest } from "@tokuly/api";
import { isHiddenSecret, type RarityKey, rarityKey, remaining, tierKey } from "@tokuly/core/quest";
import { Lock } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Text, View } from "react-native";
import { type ColorName, INK_ON_BRIGHT, sticker, useTheme } from "#/lib/theme";
import { useLocalized } from "./data";

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

const MEDAL_SIZES = { sm: 40, md: 64, xl: 128 } as const;
const TIER_FILL: Record<"bronze" | "silver" | "gold", ColorName> = {
  bronze: "coral",
  silver: "surface-3",
  gold: "sun",
};

/**
 * A badge as a round sticker: its emoji on the colour of its tier, with the
 * tier on a ribbon. One not earned is greyed with a lock; a secret one not
 * earned is a question mark.
 */
export function BadgeMedal({
  badge,
  size = "md",
  owned = badge.owned,
}: {
  badge: quest.Badge;
  size?: keyof typeof MEDAL_SIZES;
  /** Draw it as earned whatever `badge.owned` says (a showcase). */
  owned?: boolean;
}) {
  const { t } = useTranslation("quest");
  const { colors } = useTheme();
  const pixels = MEDAL_SIZES[size];
  const tier = tierKey(badge.tier);
  const hidden = isHiddenSecret(badge) && !owned;
  const fill = hidden
    ? colors.ink
    : owned
      ? colors[tier ? TIER_FILL[tier] : "mint"]
      : colors["surface-2"];
  return (
    <View importantForAccessibility="no-hide-descendants" className="items-center pb-1.5">
      <View
        className="items-center justify-center rounded-full"
        style={[
          { width: pixels, height: pixels, backgroundColor: fill },
          owned ? sticker(colors, "sm") : { borderWidth: 2, borderColor: colors.ink },
        ]}
      >
        <Text
          style={{
            fontSize: pixels * 0.48,
            lineHeight: pixels * 0.62,
            opacity: owned || hidden ? 1 : 0.35,
          }}
          className={hidden ? "font-black text-surface" : undefined}
        >
          {hidden ? "?" : badge.emoji || "🏅"}
        </Text>
      </View>
      {tier && !hidden && (
        <View
          className="absolute bottom-0 rounded-full border-2 border-ink px-1"
          style={{ backgroundColor: owned ? colors[TIER_FILL[tier]] : colors["surface-2"] }}
        >
          <Text
            className="text-[9px] font-black"
            style={{ color: owned ? INK_ON_BRIGHT : colors["ink-faint"] }}
          >
            {t(size === "sm" ? `tiersShort.${tier}` : `tiers.${tier}`)}
          </Text>
        </View>
      )}
      {!owned && !hidden && size !== "sm" && (
        <View className="absolute -top-0.5 -right-0.5 rounded-full border-2 border-ink bg-surface p-0.5">
          <Lock size={size === "xl" ? 18 : 12} color={colors["ink-soft"]} />
        </View>
      )}
    </View>
  );
}

/** A badge's name: its own (with the tier), or "シークレット" while it is still a secret. */
export function useBadgeName(): (badge: quest.Badge | undefined) => string {
  const { t } = useTranslation("quest");
  const text = useLocalized();
  return (badge) => {
    if (!badge) return "";
    const name = text(badge.name);
    if (!name) return t("badges.secretName");
    const tier = tierKey(badge.tier);
    return tier ? t("badges.nameWithTier", { name, tier: t(`tiers.${tier}`) }) : name;
  };
}

const RARITY_FILL: Record<RarityKey, ColorName> = {
  common: "surface-2",
  uncommon: "mint",
  rare: "sky",
  epic: "pink",
  legendary: "sun",
};

/** How rare a badge is, as a small pill; nothing while that is not known. */
export function RarityChip({ rarity }: { rarity: quest.BadgeRarity }) {
  const { t } = useTranslation("quest");
  const { colors } = useTheme();
  const key = rarityKey(rarity);
  if (!key) return null;
  const ink = key === "common" ? colors["ink-soft"] : key === "epic" ? "#ffffff" : INK_ON_BRIGHT;
  return (
    <View
      className="rounded-full px-2 py-0.5"
      style={{ backgroundColor: colors[RARITY_FILL[key]] }}
    >
      <Text className="text-[10px] font-black" style={{ color: ink }}>
        {t(`rarity.${key}`)}
      </Text>
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

/** "Lv.12" on sun. */
export function LevelPill({ level }: { level: number }) {
  const { t } = useTranslation("quest");
  return (
    <View className="rounded-full border-2 border-ink bg-sun px-2 py-0.5">
      <Text className="text-xs font-black" style={{ color: INK_ON_BRIGHT }}>
        {t("level.short", { level })}
      </Text>
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
