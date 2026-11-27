import type { QueryClient } from "@tanstack/query-core";
import type { quest } from "@tokuly/api";
import { questKeys as keys } from "./keys";

type MissionList = quest.ListMissionsResponse | undefined;

/** Puts a mission the server returned in its slot of today's or this week's list. */
const inSlot = (known: quest.ListMissionsResponse, mission: quest.Mission) => ({
  ...known,
  daily: known.daily.map((item) => (item.id === mission.id ? mission : item)),
  weekly: known.weekly?.id === mission.id ? mission : known.weekly,
});

/**
 * Shows a claim the server answered: the mission claimed (and gone from the
 * older unclaimed ones), the day's bonus and the level after it. The list is
 * read again; the coins are the wallet's to show.
 */
export function showClaim(
  queryClient: QueryClient,
  response: quest.ClaimMissionResponse,
  asked: quest.Mission,
): void {
  const mission = response.mission ?? asked;
  queryClient.setQueryData<MissionList>(
    keys.missions,
    (known) =>
      known && {
        ...inSlot(known, mission),
        unclaimed: known.unclaimed.filter((item) => item.id !== mission.id),
        dailyBonus: response.dailyBonus ?? known.dailyBonus,
      },
  );
  if (response.progress) queryClient.setQueryData(keys.progress, response.progress);
  void queryClient.invalidateQueries({ queryKey: keys.missions });
}

/** Shows the mission that took a rerolled one's slot, and the rerolls left. */
export function showReroll(queryClient: QueryClient, response: quest.RerollMissionResponse): void {
  const mission = response.mission;
  if (!mission) return;
  queryClient.setQueryData<MissionList>(
    keys.missions,
    (known) => known && { ...inSlot(known, mission), rerollsLeft: response.rerollsLeft },
  );
}

/** What changes for every view of badges when the caller's showcase or title changes. */
export function badgesChanged(queryClient: QueryClient): void {
  void queryClient.invalidateQueries({ queryKey: keys.badges });
  void queryClient.invalidateQueries({ queryKey: keys.progress });
  void queryClient.invalidateQueries({ queryKey: keys.profiles });
}

/**
 * Applies one realtime quest event to the query cache: what is on screen
 * moves at once, and what the event only hints at is read again.
 */
export function applyQuestEvent(queryClient: QueryClient, event: quest.QuestEvent): void {
  const setLevel = (level: quest.LevelProgress | undefined, xpToday?: number) => {
    if (!level) return;
    queryClient.setQueryData<quest.Progress | undefined>(keys.progress, (known) =>
      known ? { ...known, level, xpToday: xpToday ?? known.xpToday } : known,
    );
  };
  switch (event.event.case) {
    case "xpGained": {
      const gained = event.event.value;
      setLevel(gained.level, gained.xpToday);
      // Per-source numbers, mission progress and rankings follow the XP.
      void queryClient.invalidateQueries({ queryKey: keys.progress });
      void queryClient.invalidateQueries({ queryKey: keys.missions });
      void queryClient.invalidateQueries({ queryKey: keys.history });
      void queryClient.invalidateQueries({ queryKey: keys.leaderboards });
      void queryClient.invalidateQueries({ queryKey: keys.season });
      return;
    }
    case "levelUp":
      setLevel(event.event.value.progress);
      void queryClient.invalidateQueries({ queryKey: keys.profiles });
      return;
    case "badgeUnlocked":
      badgesChanged(queryClient);
      return;
    case "missionCompleted": {
      const mission = event.event.value;
      queryClient.setQueryData<MissionList>(
        keys.missions,
        (known) => known && inSlot(known, mission),
      );
      void queryClient.invalidateQueries({ queryKey: keys.missions });
      return;
    }
    case "streakChanged":
      void queryClient.invalidateQueries({ queryKey: keys.streaks });
      void queryClient.invalidateQueries({ queryKey: keys.profiles });
      void queryClient.invalidateQueries({ queryKey: keys.leaderboards });
      return;
    default:
      return;
  }
}
