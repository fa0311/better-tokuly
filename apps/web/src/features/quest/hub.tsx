import { Link } from "@tanstack/react-router";
import { quest } from "@tokuly/api";
import { claimable, levelFraction, xpToNextLevel } from "@tokuly/core/quest";
import { ChevronRight, Trophy } from "lucide-react";
import { useTranslation } from "react-i18next";
import i18n from "#/lib/i18n";
import type { HubTile } from "#/seams/hub";
import { useMissions, useProgress } from "./api";
import { ProgressBar } from "./parts";

/**
 * The quest tile of the "じぶん" hub: the level with its bar, and how today's
 * missions stand (with a nudge when a reward waits).
 */
function QuestHubTile() {
  const { t } = useTranslation("quest");
  const { data: progress } = useProgress();
  const { data: missions } = useMissions();
  const level = progress?.level;
  const daily = missions?.daily ?? [];
  const done = daily.filter((mission) => mission.state !== quest.MissionState.ACTIVE).length;
  const waiting = claimable(missions).length;
  return (
    <Link
      to="/quest"
      aria-label={t("title")}
      className="pressable sticker flex items-center gap-4 rounded-card bg-surface p-4"
    >
      <span className="grid size-11 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-sun text-[#17122b]">
        {level ? (
          <span className="text-center leading-none">
            <span className="block text-[9px] font-black">Lv.</span>
            <span className="block text-lg font-black tabular-nums">{level.level}</span>
          </span>
        ) : (
          <Trophy className="size-[22px]" />
        )}
      </span>
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-2">
          <span className="font-black">{t("title")}</span>
          {waiting > 0 && (
            <span className="rounded-full bg-pink px-2 py-0.5 text-[11px] font-black text-white">
              {t("tile.claim", { count: waiting })}
            </span>
          )}
        </span>
        <span className="block truncate text-sm font-bold text-ink-soft">
          {level
            ? t("tile.line", {
                xp: xpToNextLevel(level).toLocaleString(),
                done,
                total: daily.length,
              })
            : t("tile.empty")}
        </span>
        {level && (
          <ProgressBar
            className="mt-1.5 h-2 border-0 bg-surface-3"
            value={levelFraction(level)}
            label={t("home.levelBar")}
          />
        )}
      </span>
      <ChevronRight className="size-6 shrink-0 text-ink-faint" />
    </Link>
  );
}

/** The way in to missions, badges, streaks and rankings from the hub. */
export default [
  {
    id: "quest",
    section: "play",
    order: 10,
    to: "/quest",
    label: () => i18n.t("quest:title"),
    icon: Trophy,
    tint: "bg-sun",
    badge: "quest",
    keywords: [
      "quest",
      "mission",
      "badge",
      "level",
      "streak",
      "ranking",
      "ミッション",
      "バッジ",
      "レベル",
      "ストリーク",
      "ランキング",
      "称号",
    ],
    Card: QuestHubTile,
  },
] satisfies HubTile[];
