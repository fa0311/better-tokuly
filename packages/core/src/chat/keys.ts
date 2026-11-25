import { chat } from "@tokuly/api";

/** The three lists a conversation can be in. */
export type ListFilter = "inbox" | "requests" | "archived";

export const LIST_FILTERS: Record<ListFilter, chat.ConversationFilter> = {
  inbox: chat.ConversationFilter.INBOX,
  requests: chat.ConversationFilter.REQUESTS,
  archived: chat.ConversationFilter.ARCHIVED,
};

/** Query keys of the chat domain. */
export const chatKeys = {
  all: ["chat"] as const,
  capabilities: ["chat", "capabilities"] as const,
  unread: ["chat", "unread"] as const,
  /** Prefix of the three conversation lists. */
  lists: ["chat", "list"] as const,
  list: (filter: ListFilter) => ["chat", "list", filter] as const,
  /** Prefix of every single-conversation entry. */
  conversations: ["chat", "conversation"] as const,
  conversation: (id: string) => ["chat", "conversation", id] as const,
  /** Prefix of every message window. */
  timelines: ["chat", "timeline"] as const,
  timeline: (id: string) => ["chat", "timeline", id] as const,
  members: (id: string) => ["chat", "members", id] as const,
  pinned: (id: string) => ["chat", "pinned", id] as const,
  media: (id: string, filter: chat.SharedMediaFilter) => ["chat", "media", id, filter] as const,
  search: (id: string, query: string) => ["chat", "search", id, query] as const,
  reactions: (messageId: string, emoji: string) => ["chat", "reactions", messageId, emoji] as const,
  receipts: (messageId: string) => ["chat", "receipts", messageId] as const,
  calls: (missedOnly: boolean) => ["chat", "calls", missedOnly] as const,
  suggestions: (id: string, lastSeq: string) => ["chat", "suggestions", id, lastSeq] as const,
};
