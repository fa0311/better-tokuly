/**
 * The notification categories of the server (`CATEGORIES` in the notify
 * crate), in its order. A push names its category; Android shows it in the
 * channel of that name, so the phone makes one channel for each.
 */
export const NOTIFY_CATEGORIES = [
  "social",
  "chat",
  "geo",
  "notes",
  "video",
  "live",
  "spaces",
  "shorts",
  "engagement",
  "stamp",
  "wallet",
  "ask",
  "plans",
  "school",
  "community",
  "quest",
  "feed",
  "system",
] as const;

export type NotifyCategory = (typeof NOTIFY_CATEGORIES)[number];
