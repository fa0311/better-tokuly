export const INBOX_TABS = ["inbox", "answered", "archived", "filtered"] as const;
export type InboxTab = (typeof INBOX_TABS)[number];
