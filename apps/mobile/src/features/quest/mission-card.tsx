import { quest, toMillis } from "@tokuly/api";
import { difficultyKey, fraction, missionPath } from "@tokuly/core/quest";
import { router } from "expo-router";
import { ArrowRight, Check, Gift, Shuffle } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Button } from "#/components/button";
import { webScreenHref } from "#/features/web/handoff";
import { type ColorName, INK_ON_BRIGHT, sticker, useTheme } from "#/lib/theme";
import { screenFor } from "#/seams/links";
import { useLocalized } from "./data";
import { ProgressBar, RewardLine, useRemainingText } from "./parts";

const DIFFICULTY_FILL: Record<"easy" | "normal" | "hard", ColorName> = {
  easy: "mint",
  normal: "sky",
  hard: "pink",
};

/** Where a mission is done: its screen here, else the web screen. */
const openPath = (path: string) => router.push((screenFor(path) ?? webScreenHref(path)) as never);

/**
 * One mission: what to do, how far it is, what it gives, and what can be
 * done with it now — claim, go and do it, or swap it (`onReroll`, unset once
 * the day's reroll is used).
 */
export function MissionCard({
  mission,
  now,
  claiming,
  rerolling,
  onClaim,
  onReroll,
}: {
  mission: quest.Mission;
  now: number;
  claiming: boolean;
  rerolling: boolean;
  onClaim(): void;
  onReroll?: () => void;
}) {
  const { t } = useTranslation("quest");
  const { colors } = useTheme();
  const text = useLocalized();
  const left = useRemainingText();
  const title = text(mission.title);
  const difficulty = difficultyKey(mission.difficulty);
  const active = mission.state === quest.MissionState.ACTIVE;
  const completed = mission.state === quest.MissionState.COMPLETED;
  const claimed = mission.state === quest.MissionState.CLAIMED;
  const expired = mission.state === quest.MissionState.EXPIRED;
  // A mission of an earlier period stays listed until it is claimed; it cannot be swapped.
  const current = mission.endsAt ? toMillis(mission.endsAt) > now : false;
  const path = missionPath(mission);
  const progress = t("missions.progress", {
    progress: Number(mission.progress).toLocaleString(),
    target: Number(mission.target).toLocaleString(),
  });

  return (
    <View
      testID={`mission-${mission.key}`}
      className={`gap-3 rounded-card p-4 ${completed ? "bg-brand-soft" : "bg-surface"} ${claimed || expired ? "opacity-70" : ""}`}
      style={sticker(colors)}
    >
      <View className="flex-row items-start gap-3">
        <View className="h-12 w-12 items-center justify-center rounded-2xl border-2 border-ink bg-surface-2">
          <Text className="text-2xl">{mission.emoji || "🎯"}</Text>
        </View>
        <View className="flex-1 gap-0.5">
          <View className="flex-row flex-wrap items-center gap-x-2 gap-y-1">
            <Text className="shrink font-black text-ink">{title}</Text>
            {difficulty && (
              <View
                className="rounded-full px-2 py-0.5"
                style={{ backgroundColor: colors[DIFFICULTY_FILL[difficulty]] }}
              >
                <Text
                  className="text-[11px] font-black"
                  style={{ color: difficulty === "hard" ? "#ffffff" : INK_ON_BRIGHT }}
                >
                  {t(`missions.difficulty.${difficulty}`)}
                </Text>
              </View>
            )}
          </View>
          <Text className="text-sm text-ink-soft">{text(mission.description)}</Text>
        </View>
        {active && current && onReroll && (
          <Pressable
            testID={`mission-reroll-${mission.key}`}
            accessibilityRole="button"
            accessibilityLabel={t("missions.reroll", { title })}
            disabled={rerolling}
            onPress={onReroll}
            className="h-9 w-9 items-center justify-center rounded-full active:bg-surface-2"
          >
            <Shuffle size={18} color={colors["ink-soft"]} />
          </Pressable>
        )}
      </View>

      <View className="flex-row items-center gap-3">
        <ProgressBar
          value={claimed || completed ? 1 : fraction(mission.progress, mission.target)}
          label={t("missions.progressLabel", { title })}
          tone={claimed || completed ? "lime" : "brand"}
        />
        <Text className="text-sm font-black text-ink">{progress}</Text>
      </View>

      <View className="flex-row flex-wrap items-center justify-between gap-2">
        <RewardLine reward={mission.reward} />
        {completed ? (
          <Button
            testID={`mission-claim-${mission.key}`}
            variant="lime"
            size="sm"
            loading={claiming}
            icon={<Gift size={16} color={INK_ON_BRIGHT} />}
            onPress={onClaim}
          >
            {t("missions.claim")}
          </Button>
        ) : claimed ? (
          <View testID={`mission-claimed-${mission.key}`} className="flex-row items-center gap-1">
            <Check size={16} color={colors.success} />
            <Text className="text-sm font-black text-success">{t("missions.claimedState")}</Text>
          </View>
        ) : expired ? (
          <Text className="text-sm font-bold text-ink-faint">{t("missions.expired")}</Text>
        ) : path ? (
          <Pressable
            accessibilityRole="button"
            onPress={() => openPath(path)}
            className="h-9 flex-row items-center gap-1 rounded-full px-3 active:bg-surface-2"
          >
            <Text className="text-sm font-black text-brand">{t("missions.go")}</Text>
            <ArrowRight size={16} color={colors.brand} />
          </Pressable>
        ) : (
          mission.endsAt && (
            <Text className="text-sm font-bold text-ink-faint">
              {t("missions.endsIn", { left: left(toMillis(mission.endsAt), now) })}
            </Text>
          )
        )}
      </View>
    </View>
  );
}
