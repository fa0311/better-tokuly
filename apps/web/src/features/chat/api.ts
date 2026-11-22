import { useQuery } from "@tanstack/react-query";
import { chat } from "@tokuly/api";
import { api } from "#/lib/api";

export const conversationClient = () => api().service(chat.ConversationService);
export const messageClient = () => api().service(chat.MessageService);
export const povClient = () => api().service(chat.PovService);
export const callClient = () => api().service(chat.CallService);
export const assistClient = () => api().service(chat.AssistService);

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

/** What the server allows when it has not told us yet (its own defaults). */
const DEFAULT_CAPABILITIES = {
  maxTextLength: 4000,
  maxAttachments: 10,
  maxGroupMembers: 100,
  editWindowSeconds: 900,
  requestMessageLimit: 3,
  maxPinnedMessages: 50,
  povAnswerSeconds: 7200,
  povReplySeconds: 86_400,
  callsEnabled: false,
  ringSeconds: 45,
  assistEnabled: false,
};

export type Capabilities = typeof DEFAULT_CAPABILITIES;

/**
 * Limits and optional features of this server. Until the answer is in, the
 * optional features (calls, AI) count as off, so their controls never flash.
 */
export function useCapabilities(): Capabilities {
  const { data } = useQuery({
    queryKey: chatKeys.capabilities,
    queryFn: async () => (await conversationClient().getCapabilities({})).capabilities,
    staleTime: 10 * 60_000,
  });
  if (!data) return DEFAULT_CAPABILITIES;
  return {
    maxTextLength: data.maxTextLength || DEFAULT_CAPABILITIES.maxTextLength,
    maxAttachments: data.maxAttachments || DEFAULT_CAPABILITIES.maxAttachments,
    maxGroupMembers: data.maxGroupMembers || DEFAULT_CAPABILITIES.maxGroupMembers,
    editWindowSeconds: data.editWindowSeconds || DEFAULT_CAPABILITIES.editWindowSeconds,
    requestMessageLimit: data.requestMessageLimit || DEFAULT_CAPABILITIES.requestMessageLimit,
    maxPinnedMessages: data.maxPinnedMessages || DEFAULT_CAPABILITIES.maxPinnedMessages,
    povAnswerSeconds: data.povAnswerSeconds || DEFAULT_CAPABILITIES.povAnswerSeconds,
    povReplySeconds: data.povReplySeconds || DEFAULT_CAPABILITIES.povReplySeconds,
    callsEnabled: data.callsEnabled,
    ringSeconds: data.ringSeconds || DEFAULT_CAPABILITIES.ringSeconds,
    assistEnabled: data.assistEnabled,
  };
}
