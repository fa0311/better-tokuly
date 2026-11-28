import type { plans } from "@tokuly/api";

/** Query keys of the plans domain; all start with `plans`, so one invalidation refreshes everything. */
export const planKeys = {
  all: ["plans"] as const,
  free: ["plans", "free"] as const,
  freeFriends: ["plans", "free", "friends"] as const,
  freeAnswers: ["plans", "free", "answers"] as const,
  freeAlerts: ["plans", "free", "alerts"] as const,
  freeStatuses: ["plans", "free", "status"] as const,
  /** One person's status; the caller's own under the empty ID. */
  freeStatus: (userId: string) => ["plans", "free", "status", userId] as const,
  planLists: ["plans", "list"] as const,
  planList: (filter: plans.PlanListFilter) => ["plans", "list", filter] as const,
  plan: (planId: string) => ["plans", "plan", planId] as const,
  expenses: (planId: string) => ["plans", "expenses", planId] as const,
  split: (planId: string) => ["plans", "split", planId] as const,
  polls: ["plans", "polls"] as const,
  everyPoll: ["plans", "poll"] as const,
  /** `pollId` is whatever the address names: the ID or the short ID of the link. */
  poll: (pollId: string) => ["plans", "poll", pollId] as const,
  countdowns: ["plans", "countdowns"] as const,
  countdown: (countdownId: string) => ["plans", "countdown", countdownId] as const,
  anniversaries: ["plans", "anniversaries"] as const,
  birthdays: ["plans", "birthdays"] as const,
  settings: ["plans", "settings"] as const,
  calendar: (month: string) => ["plans", "calendar", month] as const,
  calendars: ["plans", "calendar"] as const,
};
