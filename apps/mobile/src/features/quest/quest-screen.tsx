import { type quest, toMillis } from "@tokuly/api";
import { levelFraction, xpToNextLevel } from "@tokuly/core/quest";
import { router } from "expo-router";
import {
  Award,
  Flame,
  Gift,
  ListChecks,
  Settings2,
  Shuffle,
  Sparkles,
  Trophy,
} from "lucide-react-native";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Text, View } from "react-native";
import { Button } from "#/components/button";
import { EmptyState, Page, Row, Section, Spinner } from "#/components/page";
import { webScreenHref } from "#/features/web/handoff";
import { INK_ON_BRIGHT, sticker, useTheme } from "#/lib/theme";
import { useClaim, useReroll } from "./actions";
import { useCoins, useLocalized, useMissions, useProgress } from "./data";
import { MissionCard } from "./mission-card";
import { ProgressBar, RewardLine, TitlePill, useRemainingText } from "./parts";

/** The rest of the quest area, on the web for now. */
const WEB_PAGES = [
  { key: "streaks", label: "nav.streaks", path: "/quest/streaks", Icon: Flame },
  { key: "ranking", label: "nav.ranking", path: "/quest/ranking", Icon: Trophy },
  { key: "xp", label: "xpGuide.title", path: "/quest/xp", Icon: ListChecks },
  { key: "showcase", label: "showcase.title", path: "/quest/showcase", Icon: Sparkles },
  { key: "settings", label: "settings.title", path: "/quest/settings", Icon: Settings2 },
] as const;

/** The clock, a tick every half minute: enough for "あと5時間". */
function useNow(): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(timer);
  }, []);
  return now;
}

/**
 * `/quest`: where the player stands (level, XP, title, coins), today's and
 * this week's missions with their rewards, the bonus for finishing the day,
 * the day's reroll, rewards of earlier days still waiting, and the badges.
 */
export function QuestScreen() {
  const { t } = useTranslation(["quest", "mobile"]);
  const { colors } = useTheme();
  const progress = useProgress();
  return (
    <Page testID="quest-screen">
      <LevelCard />
      <Missions />
      <Section>
        <Row
          testID="quest-badges"
          icon={<Award size={20} color={colors.ink} />}
          label={t("quest:nav.badges")}
          hint={
            progress.data
              ? t("mobile:quest.badgesHint", { count: progress.data.badgeCount })
              : undefined
          }
          onPress={() => router.push(webScreenHref("/quest/badges") as never)}
        />
      </Section>
      <Section title={t("mobile:quest.onWeb")}>
        {WEB_PAGES.map(({ key, label, path, Icon }) => (
          <Row
            key={key}
            testID={`quest-web-${key}`}
            icon={<Icon size={20} color={colors.ink} />}
            label={t(`quest:${label}`)}
            onPress={() => router.push(webScreenHref(path) as never)}
          />
        ))}
      </Section>
    </Page>
  );
}

/** The level with the bar to the next one, the title worn, and the coins (the wallet's number). */
function LevelCard() {
  const { t } = useTranslation(["quest", "mobile", "common"]);
  const { colors } = useTheme();
  const text = useLocalized();
  const { data: progress, isPending, isError, refetch } = useProgress();
  const coins = useCoins();

  if (isPending) return <Spinner />;
  if (isError || !progress?.level)
    return (
      <EmptyState
        emoji="🛰️"
        title={t("home.error")}
        action={
          <Button variant="secondary" onPress={() => void refetch()}>
            {t("common:actions.retry")}
          </Button>
        }
      />
    );

  const level = progress.level;
  const title = text(progress.title?.text);
  return (
    <View className="overflow-hidden rounded-card bg-surface" style={sticker(colors)}>
      <View className="gap-3 bg-sun p-4">
        <View className="flex-row items-center gap-4">
          <View
            className="h-20 w-20 items-center justify-center rounded-full bg-surface"
            style={sticker(colors, "sm")}
            accessibilityLabel={t("level.label", { level: level.level })}
          >
            <Text className="text-[11px] font-black text-ink">Lv.</Text>
            <Text testID="quest-level" className="text-4xl font-black text-ink">
              {level.level}
            </Text>
          </View>
          <View className="flex-1 gap-1">
            {title ? <TitlePill title={title} /> : null}
            <Text className="text-2xl font-black" style={{ color: INK_ON_BRIGHT }}>
              {t("home.totalXp", { xp: Number(level.totalXp).toLocaleString() })}
            </Text>
            <Text className="text-sm font-bold" style={{ color: INK_ON_BRIGHT }}>
              {t("home.toNext", {
                xp: xpToNextLevel(level).toLocaleString(),
                level: level.level + 1,
              })}
            </Text>
          </View>
        </View>
        <View className="flex-row">
          <ProgressBar value={levelFraction(level)} label={t("home.levelBar")} />
        </View>
        <Text className="text-right text-xs font-black" style={{ color: INK_ON_BRIGHT }}>
          {t("home.levelBarValue", {
            into: Number(level.xpIntoLevel).toLocaleString(),
            needed: Number(level.xpForNextLevel).toLocaleString(),
          })}
        </Text>
      </View>
      <View className="flex-row border-t-2 border-ink">
        <Stat label={t("home.today")} value={t("xp.amount", { xp: progress.xpToday })} />
        <Stat divided label={t("home.badges")} value={progress.badgeCount.toLocaleString()} />
        <Stat
          divided
          testID="quest-coins"
          label={t("mobile:quest.coins")}
          value={coins.data === undefined ? "…" : `🪙 ${coins.data.toLocaleString()}`}
        />
      </View>
    </View>
  );
}

function Stat({
  label,
  value,
  divided,
  testID,
}: {
  label: string;
  value: string;
  /** A line on its left, between it and the one before. */
  divided?: boolean;
  testID?: string;
}) {
  return (
    <View className={`flex-1 items-center px-2 py-3 ${divided ? "border-l-2 border-line" : ""}`}>
      <Text testID={testID} className="text-lg font-black text-ink">
        {value}
      </Text>
      <Text className="text-xs font-bold text-ink-faint">{label}</Text>
    </View>
  );
}

function Heading({ title, aside }: { title: string; aside?: string }) {
  return (
    <View className="flex-row items-baseline justify-between gap-3 px-1">
      <Text className="text-lg font-black text-ink">{title}</Text>
      {aside ? <Text className="text-xs font-bold text-ink-soft">{aside}</Text> : null}
    </View>
  );
}

/** Today's and this week's missions, and what earlier days left to collect. */
function Missions() {
  const { t } = useTranslation(["quest", "common"]);
  const { colors } = useTheme();
  const { data, isPending, isError, refetch } = useMissions();
  const now = useNow();
  const left = useRemainingText();
  const { claim, claiming } = useClaim();
  const reroll = useReroll();

  if (isPending) return <Spinner />;
  if (isError || !data)
    return (
      <EmptyState
        emoji="🛰️"
        title={t("missions.error")}
        action={
          <Button variant="secondary" onPress={() => void refetch()}>
            {t("common:actions.retry")}
          </Button>
        }
      />
    );

  const canReroll = data.rerollsLeft > 0;
  const card = (mission: quest.Mission) => (
    <MissionCard
      key={mission.id}
      mission={mission}
      now={now}
      claiming={claiming === mission.id}
      rerolling={reroll.rerolling === mission.id}
      onClaim={() => claim(mission)}
      onReroll={canReroll ? () => reroll.ask(mission) : undefined}
    />
  );
  const bonus = data.dailyBonus;
  const resets = (at: typeof data.dailyResetsAt) =>
    at ? t("missions.resetsIn", { left: left(toMillis(at), now) }) : undefined;

  return (
    <>
      {data.unclaimed.length > 0 && (
        <View className="gap-3">
          <Heading title={t("missions.unclaimed")} aside={t("missions.unclaimedHint")} />
          {data.unclaimed.map(card)}
        </View>
      )}

      <View className="gap-3">
        <Heading title={t("missions.daily")} aside={resets(data.dailyResetsAt)} />
        {data.daily.length === 0 ? (
          <EmptyState emoji="🌙" title={t("missions.none")} body={t("missions.noneBody")} />
        ) : (
          data.daily.map(card)
        )}
        {bonus && bonus.required > 0 && (
          <View
            testID="quest-bonus"
            className={`flex-row items-center gap-3 rounded-card border-2 border-dashed border-ink px-4 py-3 ${bonus.granted ? "bg-lime" : "bg-surface"}`}
          >
            <Gift size={24} color={bonus.granted ? INK_ON_BRIGHT : colors.ink} />
            <View className="flex-1">
              <Text
                className="font-black"
                style={{ color: bonus.granted ? INK_ON_BRIGHT : colors.ink }}
              >
                {bonus.granted ? t("missions.bonusGranted") : t("missions.bonus")}
              </Text>
              <Text
                className="text-sm font-bold"
                style={{ color: bonus.granted ? INK_ON_BRIGHT : colors["ink-soft"] }}
              >
                {t("missions.bonusProgress", { claimed: bonus.claimed, required: bonus.required })}
              </Text>
            </View>
            <RewardLine reward={bonus.reward} />
          </View>
        )}
        <View className="flex-row items-center gap-1.5 px-1">
          <Shuffle size={16} color={colors["ink-soft"]} />
          <Text className="flex-1 text-sm font-bold text-ink-soft">
            {canReroll ? t("missions.rerollLeft") : t("missions.rerollUsed")}
          </Text>
        </View>
      </View>

      {data.weekly && (
        <View className="gap-3">
          <Heading title={t("missions.weekly")} aside={resets(data.weeklyResetsAt)} />
          {card(data.weekly)}
        </View>
      )}
    </>
  );
}
