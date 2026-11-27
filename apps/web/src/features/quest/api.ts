import { useQuery } from "@tanstack/react-query";
import { type common, quest } from "@tokuly/api";
import { localized, questQueries } from "@tokuly/core/quest";
import { useTranslation } from "react-i18next";
import { api } from "#/lib/api";

export const questClient = () => api().service(quest.QuestService);
export const missionClient = () => api().service(quest.MissionService);
export const badgeClient = () => api().service(quest.BadgeService);
export const streakClient = () => api().service(quest.StreakService);
export const leaderboardClient = () => api().service(quest.LeaderboardService);
export const questAdminClient = () => api().service(quest.QuestAdminService);

/** `localized` bound to the language the app is shown in. */
export function useLocalized(): (text: common.LocalizedText | undefined) => string {
  const { i18n } = useTranslation();
  return (text) => localized(text, i18n.language);
}

/** The signed-in player's level, XP and what they earned today. */
export function useProgress() {
  return useQuery(questQueries.progress(questClient));
}

/** Today's and this week's missions. */
export function useMissions() {
  return useQuery(questQueries.missions(missionClient));
}

/** Every badge by family, with what the signed-in player owns. */
export function useBadgeCatalog() {
  return useQuery(questQueries.catalog(badgeClient));
}

/** A player's badges, showcase and title (`""` = the signed-in player). */
export function useUserBadges(userId: string, enabled = true) {
  return useQuery({ ...questQueries.userBadges(badgeClient, userId), enabled });
}

/**
 * What a person shows of their quest progress (kept five minutes: chips sit
 * in lists). `null` when there is no such person for the viewer.
 */
export function useQuestProfile(userId: string | undefined) {
  return useQuery({
    ...questQueries.profile(questClient, userId ?? ""),
    enabled: Boolean(userId),
  });
}

/**
 * The first page of the signed-in player's streaks. The flame next to a
 * friend's name reads this list, so a screen full of friends asks once.
 */
export function useStreakList(enabled = true) {
  return useQuery({ ...questQueries.streakList(streakClient), enabled });
}
