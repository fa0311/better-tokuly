import type { InboxFilter } from "./inbox-cache";

/** Query keys of the notify domain. Everything live-updated hangs under `inbox` and `unread`. */
export const notifyKeys = {
  all: ["notify"] as const,
  inbox: ["notify", "inbox"] as const,
  inboxList: (filter: InboxFilter) => ["notify", "inbox", filter] as const,
  unread: ["notify", "unread"] as const,
  preferences: ["notify", "preferences"] as const,
  muted: ["notify", "muted"] as const,
  pushConfig: ["notify", "push", "config"] as const,
  devices: ["notify", "push", "devices"] as const,
};
