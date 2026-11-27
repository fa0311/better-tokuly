export const MODERATION_TABS = ["queue", "received", "mine", "rules", "blocked"] as const;
export type ModerationTab = (typeof MODERATION_TABS)[number];
