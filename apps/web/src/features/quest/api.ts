import { type QueryClient, useQuery } from "@tanstack/react-query";
import { type common, errorReason, quest } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { api } from "#/lib/api";

export const questClient = () => api().service(quest.QuestService);
export const missionClient = () => api().service(quest.MissionService);
export const badgeClient = () => api().service(quest.BadgeService);
export const streakClient = () => api().service(quest.StreakService);
export const leaderboardClient = () => api().service(quest.LeaderboardService);
export const questAdminClient = () => api().service(quest.QuestAdminService);

/** Query keys: everything here starts with `["quest"]`, so one invalidation refreshes the lot. */
export const questKeys = {
  all: ["quest"] as const,
  progress: ["quest", "progress"] as const,
  history: ["quest", "history"] as const,
  rules: ["quest", "rules"] as const,
  preferences: ["quest", "preferences"] as const,
  missions: ["quest", "missions"] as const,
  badges: ["quest", "badges"] as const,
  catalog: ["quest", "badges", "catalog"] as const,
  badge: (key: string) => ["quest", "badges", "one", key] as const,
  userBadges: (userId: string) => ["quest", "badges", "user", userId] as const,
  profiles: ["quest", "profile"] as const,
  profile: (userId: string) => ["quest", "profile", userId] as const,
  streaks: ["quest", "streaks"] as const,
  streakList: ["quest", "streaks", "list"] as const,
  streak: (userId: string) => ["quest", "streaks", "with", userId] as const,
  boards: ["quest", "boards"] as const,
  leaderboards: ["quest", "leaderboard"] as const,
  leaderboard: (board: string, period: quest.LeaderboardPeriod) =>
    ["quest", "leaderboard", board, period] as const,
  season: ["quest", "season"] as const,
  seasonResults: ["quest", "season", "results"] as const,
  admin: {
    actions: (userId: string) => ["quest", "admin", "actions", userId] as const,
  },
};

/** A text the server sends in both languages, in the one the app is shown in. */
export function localized(text: common.LocalizedText | undefined, language: string): string {
  if (!text) return "";
  return (language.startsWith("ja") ? text.ja || text.en : text.en || text.ja) ?? "";
}

/** `localized` bound to the language the app is shown in. */
export function useLocalized(): (text: common.LocalizedText | undefined) => string {
  const { i18n } = useTranslation();
  return (text) => localized(text, i18n.language);
}

/** The signed-in player's level, XP and what they earned today. */
export function useProgress() {
  return useQuery({
    queryKey: questKeys.progress,
    queryFn: async () => (await questClient().getProgress({})).progress,
  });
}

/** Today's and this week's missions. */
export function useMissions() {
  return useQuery({
    queryKey: questKeys.missions,
    queryFn: () => missionClient().listMissions({}),
  });
}

/** Every badge by family, with what the signed-in player owns. */
export function useBadgeCatalog() {
  return useQuery({
    queryKey: questKeys.catalog,
    queryFn: () => badgeClient().listBadgeCatalog({}),
  });
}

/** A player's badges, showcase and title (`""` = the signed-in player). */
export function useUserBadges(userId: string, enabled = true) {
  return useQuery({
    queryKey: questKeys.userBadges(userId),
    queryFn: () => badgeClient().listUserBadges({ userId }),
    enabled,
  });
}

/** How long what a person shows of their progress is trusted: chips sit in lists and must not ask again per row. */
const PROFILE_STALE_MS = 5 * 60_000;

/**
 * What a person shows of their quest progress. `null` when there is no such
 * person for the viewer (deleted, or one of the two blocked the other).
 */
export function useQuestProfile(userId: string | undefined) {
  return useQuery({
    queryKey: questKeys.profile(userId ?? ""),
    queryFn: async () => {
      try {
        return (await questClient().getProfile({ userId: userId ?? "" })).profile ?? null;
      } catch (error) {
        if (errorReason(error) === "USER_NOT_FOUND") return null;
        throw error;
      }
    },
    enabled: Boolean(userId),
    staleTime: PROFILE_STALE_MS,
    retry: false,
  });
}

/** Streaks asked for per page: enough that the first page holds every running streak of nearly everyone. */
export const STREAK_PAGE_SIZE = 100;

/**
 * The first page of the signed-in player's streaks (longest first, then the
 * ones lost within a day), with what a restore costs. The flame next to a
 * friend's name reads this list, so a screen full of friends asks once.
 */
export function useStreakList(enabled = true) {
  return useQuery({
    queryKey: questKeys.streakList,
    queryFn: () => streakClient().listStreaks({ page: { limit: STREAK_PAGE_SIZE } }),
    enabled,
  });
}

/**
 * Applies one realtime quest event to the query cache: what is on screen
 * moves at once, and what the event only hints at is read again.
 */
export function applyQuestEvent(queryClient: QueryClient, event: quest.QuestEvent): void {
  const setLevel = (level: quest.LevelProgress | undefined, xpToday?: number) => {
    if (!level) return;
    queryClient.setQueryData<quest.Progress | undefined>(questKeys.progress, (known) =>
      known ? { ...known, level, xpToday: xpToday ?? known.xpToday } : known,
    );
  };
  switch (event.event.case) {
    case "xpGained": {
      const gained = event.event.value;
      setLevel(gained.level, gained.xpToday);
      // Per-source numbers, mission progress and rankings follow the XP.
      void queryClient.invalidateQueries({ queryKey: questKeys.progress });
      void queryClient.invalidateQueries({ queryKey: questKeys.missions });
      void queryClient.invalidateQueries({ queryKey: questKeys.history });
      void queryClient.invalidateQueries({ queryKey: questKeys.leaderboards });
      void queryClient.invalidateQueries({ queryKey: questKeys.season });
      return;
    }
    case "levelUp":
      setLevel(event.event.value.progress);
      void queryClient.invalidateQueries({ queryKey: questKeys.profiles });
      return;
    case "badgeUnlocked":
      void queryClient.invalidateQueries({ queryKey: questKeys.badges });
      void queryClient.invalidateQueries({ queryKey: questKeys.progress });
      void queryClient.invalidateQueries({ queryKey: questKeys.profiles });
      return;
    case "missionCompleted": {
      const mission = event.event.value;
      queryClient.setQueryData<quest.ListMissionsResponse | undefined>(
        questKeys.missions,
        (known) =>
          known
            ? {
                ...known,
                daily: known.daily.map((item) => (item.id === mission.id ? mission : item)),
                weekly: known.weekly?.id === mission.id ? mission : known.weekly,
              }
            : known,
      );
      void queryClient.invalidateQueries({ queryKey: questKeys.missions });
      return;
    }
    case "streakChanged":
      void queryClient.invalidateQueries({ queryKey: questKeys.streaks });
      void queryClient.invalidateQueries({ queryKey: questKeys.profiles });
      void queryClient.invalidateQueries({ queryKey: questKeys.leaderboards });
      return;
    default:
      return;
  }
}
