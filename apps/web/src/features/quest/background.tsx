import { useQueryClient } from "@tanstack/react-query";
import { useRouter } from "@tanstack/react-router";
import { quest } from "@tokuly/api";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { openDirectChat } from "#/features/chat";
import { celebrate as rainGlyphs } from "#/features/footprints";
import { realtime } from "#/lib/realtime";
import { useSessionStore } from "#/stores/session";
import { applyQuestEvent, localized, questKeys } from "./api";
import { clearReveals, motionFactor, RevealLayer, reveal } from "./celebrate";
import { loudness } from "./model";

/** Away from the quest pages, XP is announced at most this often (with everything earned since). */
const XP_TOAST_EVERY_MS = 20_000;

/**
 * Badges that come with signing up. They are pushed like any other, but a
 * new account must not be interrupted on whatever it is doing: they are only
 * celebrated where badges live.
 */
const QUIET_BADGES = new Set(["welcome"]);

/**
 * Runs while someone is signed in: XP, level-ups, badges, finished missions
 * and streak changes arrive as realtime events, go into the query cache and
 * are celebrated in proportion to the ドパガキ度 — a quiet toast at the low
 * end, a card with confetti in the middle, the whole screen at the top.
 * Nothing here takes input away from the page; several moments queue.
 */
export default function QuestBackground() {
  const queryClient = useQueryClient();
  const router = useRouter();
  const { t, i18n } = useTranslation("quest");

  useEffect(() => {
    const client = realtime();
    let connectedBefore = client.connectionStatus === "connected";
    let pendingXp = 0;
    let lastXpToast = 0;

    const go = (to: "/quest" | "/quest/streaks") => () => void router.navigate({ to });
    const text = (value: Parameters<typeof localized>[0]) => localized(value, i18n.language);

    const offFrame = client.onFrame((frame) => {
      if (frame.body.case === "welcome") {
        // Pushes sent while there was no connection are lost: read again.
        if (connectedBefore) void queryClient.invalidateQueries({ queryKey: questKeys.all });
        connectedBefore = true;
        return;
      }
      if (frame.body.case !== "quest") return;
      const event = frame.body.value;
      applyQuestEvent(queryClient, event);

      const onQuest = router.state.location.pathname.startsWith("/quest");
      const loud = loudness(motionFactor());

      switch (event.event.case) {
        case "xpGained": {
          pendingXp += event.event.value.xp;
          if (pendingXp <= 0) return;
          // Earning is constant while chatting: only the quest pages (and lively settings, now and then) say it.
          const due = loud !== "toast" && Date.now() - lastXpToast >= XP_TOAST_EVERY_MS;
          if (!onQuest && !due) return;
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
                  onClick: () => void openDirectChat(friend.id),
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
    });
    client.start();

    // The cache belongs to one account: forget it when that account leaves.
    let userId = signedInUserId(useSessionStore.getState());
    const offSession = useSessionStore.subscribe((session) => {
      const next = signedInUserId(session);
      if (next === userId) return;
      if (userId) {
        queryClient.removeQueries({ queryKey: questKeys.all });
        clearReveals();
        pendingXp = 0;
      }
      userId = next;
    });

    return () => {
      offFrame();
      offSession();
    };
  }, [queryClient, router, t, i18n]);

  return <RevealLayer />;
}

function signedInUserId(session: ReturnType<typeof useSessionStore.getState>): string | undefined {
  return session.state.status === "signedIn" ? session.state.me.user?.id : undefined;
}
