import type { QueryClient } from "@tanstack/react-query";
import type { AnyRouter } from "@tanstack/react-router";
import { quest } from "@tokuly/api";
import { applyQuestEvent, localized, loudness } from "@tokuly/core/quest";
import type { TFunction } from "i18next";
import { toast } from "sonner";
import { celebrate as rainGlyphs } from "#/features/footprints";
import { errorMessage } from "#/lib/errors";
import { loadNow } from "#/lib/load-module";
import { motionFactor, reveal } from "./celebrate";

/** Away from the quest pages, XP is announced at most this often (with everything earned since). */
const XP_TOAST_EVERY_MS = 20_000;

/**
 * Badges that come with signing up. They are pushed like any other, but a
 * new account must not be interrupted on whatever it is doing: they are only
 * celebrated where badges live.
 */
const QUIET_BADGES = new Set(["welcome"]);

/**
 * What quest does with its realtime events: XP, level-ups, badges, finished
 * missions and streak changes go into the query cache and are celebrated in
 * proportion to the ドパガキ度. `background.tsx` loads this with the first one;
 * `forget` drops the XP not announced yet when the account leaves.
 */
export function questCelebrations({
  queryClient,
  router,
  t,
  language,
}: {
  queryClient: QueryClient;
  router: AnyRouter;
  t: TFunction<"quest">;
  language: () => string;
}): { apply: (event: quest.QuestEvent) => void; forget: () => void } {
  let pendingXp = 0;
  let lastXpToast = 0;

  const go = (to: "/quest" | "/quest/streaks") => () => void router.navigate({ to });
  const text = (value: Parameters<typeof localized>[0]) => localized(value, language());

  const apply = (event: quest.QuestEvent) => {
    applyQuestEvent(queryClient, event);

    const onQuest = router.state.location.pathname.startsWith("/quest");
    const loud = loudness(motionFactor());

    switch (event.event.case) {
      case "xpGained": {
        const gained = event.event.value.xp;
        if (gained <= 0) return;
        // Earning is constant while chatting: only the quest pages say it, and lively settings
        // now and then elsewhere. XP nobody will be told about is not saved up for later.
        if (!onQuest && loud === "toast") {
          pendingXp = 0;
          return;
        }
        pendingXp += gained;
        if (!onQuest && Date.now() - lastXpToast < XP_TOAST_EVERY_MS) return;
        lastXpToast = Date.now();
        toast(t("live.xp", { xp: pendingXp }), { id: "quest-xp", icon: "⚡", duration: 2500 });
        pendingXp = 0;
        return;
      }
      case "levelUp": {
        const { level, coins } = event.event.value;
        if (reveal({ kind: "level", level, coins }) === "toast")
          toast.success(t("live.levelUp", { level }), {
            description: coins > 0 ? t("live.levelUpCoins", { coins }) : undefined,
            action: onQuest ? undefined : { label: t("live.open"), onClick: go("/quest") },
          });
        return;
      }
      case "badgeUnlocked": {
        const badge = event.event.value;
        if (QUIET_BADGES.has(badge.key) && !onQuest) return;
        if (reveal({ kind: "badge", badge }) === "toast")
          toast.success(
            t("live.badge", {
              emoji: badge.emoji,
              name: text(badge.name) || t("badges.secretName"),
            }),
            {
              action: {
                label: t("live.open"),
                onClick: () =>
                  void router.navigate({ to: "/quest/badges/$key", params: { key: badge.key } }),
              },
            },
          );
        return;
      }
      case "missionCompleted": {
        const mission = event.event.value;
        if (loud !== "toast") rainGlyphs([mission.emoji || "🎯", "✨"]);
        toast.success(t("live.missionDone", { title: text(mission.title) }), {
          id: `quest-mission-${mission.id}`,
          action:
            router.state.location.pathname === "/quest"
              ? undefined
              : { label: t("live.claim"), onClick: go("/quest") },
        });
        return;
      }
      case "streakChanged": {
        const { streak, change } = event.event.value;
        const friend = streak?.friend;
        if (!streak || !friend) return;
        const name = friend.displayName;
        const id = `quest-streak-${friend.id}`;
        switch (change) {
          case quest.StreakChange.MILESTONE:
            if (reveal({ kind: "streak", days: streak.days, friendName: name }) === "toast")
              toast.success(t("live.streakMilestone", { name, count: streak.days }), { id });
            return;
          case quest.StreakChange.EXTENDED:
            if (onQuest || loud !== "toast")
              toast(t("live.streakExtended", { name, count: streak.days }), { id, icon: "🔥" });
            return;
          case quest.StreakChange.EXPIRING:
            toast.warning(t("live.streakExpiring", { name, count: streak.days }), {
              id,
              duration: 10_000,
              action: {
                label: t("live.sendMessage"),
                onClick: () =>
                  void loadNow(() => import("#/features/chat"))
                    .then((chat) => chat.openDirectChat(friend.id))
                    .catch((error: unknown) => toast.error(errorMessage(error))),
              },
            });
            return;
          case quest.StreakChange.LOST:
            // A streak of one day is not offered back, so there is nothing to do about it.
            if (!streak.restorableUntil) return;
            toast(t("live.streakLost", { name, count: streak.lostDays }), {
              id,
              icon: "💔",
              duration: 10_000,
              action: onQuest
                ? undefined
                : { label: t("live.restore"), onClick: go("/quest/streaks") },
            });
            return;
          case quest.StreakChange.RESTORED:
            toast.success(t("live.streakRestored", { name, count: streak.days }), { id });
            return;
          default:
            return;
        }
      }
      default:
        return;
    }
  };

  return {
    apply,
    forget: () => {
      pendingXp = 0;
    },
  };
}
