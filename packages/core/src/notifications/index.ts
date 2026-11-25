/**
 * Notification logic every app shares: query keys, unread counts (the
 * badges), the inbox's cache edits for what the reader does and what the
 * realtime connection reports, the inbox's times, and where an entry's path
 * leads. Push registration and opening a target stay in each app.
 */
export { applyInboxChange, rereadInbox, storeUnread, updateInboxLists } from "./cache";
export { NOTIFY_CATEGORIES, type NotifyCategory } from "./categories";
export {
  entriesOf,
  type InboxFilter,
  type InboxPages,
  PAGE_SIZE,
  withRead,
  withReceived,
  withRemoved,
} from "./inbox-cache";
export { notifyKeys } from "./keys";
export { resolveTarget, type Target } from "./target";
export { NOTHING_UNREAD, type Unread, unreadOf, unreadQuery } from "./unread";
export { type DayBucket, dayBucket, formatWhen, minutesToTime, timeToMinutes } from "./when";
