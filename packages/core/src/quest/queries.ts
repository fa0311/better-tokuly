import { errorReason, type quest } from "@tokuly/api";
import type { ClientSource } from "../query";
import { questKeys as keys } from "./keys";

type Quests = ClientSource<typeof quest.QuestService>;
type Missions = ClientSource<typeof quest.MissionService>;
type Badges = ClientSource<typeof quest.BadgeService>;
type Streaks = ClientSource<typeof quest.StreakService>;

/** How long what a person shows of their progress is trusted: chips sit in lists and must not ask again per row. */
const PROFILE_STALE_MS = 5 * 60_000;

/** Streaks asked for per page: enough that the first page holds every running streak of nearly everyone. */
export const STREAK_PAGE_SIZE = 100;

/** Query options of the quest domain. Each app adds its hooks around them. */
export const questQueries = {
  /** The caller's level, XP and what they earned today. */
  progress: (client: Quests) => ({
    queryKey: keys.progress,
    queryFn: async () => (await client().getProgress({})).progress,
  }),

  /** Today's and this week's missions. */
  missions: (client: Missions) => ({
    queryKey: keys.missions,
    queryFn: () => client().listMissions({}),
  }),

  /** Every badge by family, with what the caller owns. */
  catalog: (client: Badges) => ({
    queryKey: keys.catalog,
    queryFn: () => client().listBadgeCatalog({}),
  }),

  /** One badge with the caller's friends who own it. Not asked again once the server says there is none. */
  badge: (client: Badges, key: string) => ({
    queryKey: keys.badge(key),
    queryFn: () => client().getBadge({ key }),
    retry: (count: number, failure: unknown) =>
      errorReason(failure) !== "BADGE_NOT_FOUND" && count < 2,
  }),

  /** A player's badges, showcase and title (`""` = the caller). */
  userBadges: (client: Badges, userId: string) => ({
    queryKey: keys.userBadges(userId),
    queryFn: () => client().listUserBadges({ userId }),
  }),

  /**
   * What a person shows of their quest progress. `null` when there is no such
   * person for the viewer (deleted, or one of the two blocked the other).
   */
  profile: (client: Quests, userId: string) => ({
    queryKey: keys.profile(userId),
    queryFn: async () => {
      try {
        return (await client().getProfile({ userId })).profile ?? null;
      } catch (error) {
        if (errorReason(error) === "USER_NOT_FOUND") return null;
        throw error;
      }
    },
    staleTime: PROFILE_STALE_MS,
    retry: false,
  }),

  /**
   * The first page of the caller's streaks (longest first, then the ones
   * lost within a day), with what a restore costs.
   */
  streakList: (client: Streaks) => ({
    queryKey: keys.streakList,
    queryFn: () => client().listStreaks({ page: { limit: STREAK_PAGE_SIZE } }),
  }),
};
