import type { quest } from "@tokuly/api";

/** Query keys of the quest domain: everything starts with `["quest"]`, so one invalidation refreshes the lot. */
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
