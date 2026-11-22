import type { chat, common } from "@tokuly/api";
import { createContext, useContext } from "react";
import type { Capabilities } from "./api";

/** Media of the conversation opened full screen: which message's, and which item first. */
export interface MediaTarget {
  message: chat.Message;
  index: number;
}

/**
 * What every part of an open conversation needs: the conversation, who is
 * looking, and what the screen can do for a message. Provided by
 * `ConversationScreen`, so rows do not carry a dozen callbacks each.
 */
export interface ConversationContextValue {
  conversation: chat.Conversation;
  me: common.UserSummary;
  capabilities: Capabilities;
  /** The viewer may send here (not read-only, not declined). */
  writable: boolean;
  /** The viewer accepted the conversation (reactions and pins need that). */
  accepted: boolean;
  reply(message: chat.Message): void;
  edit(message: chat.Message): void;
  forward(message: chat.Message): void;
  remove(message: chat.Message): void;
  /** Scrolls to a message, loading the part of the conversation it is in when needed. */
  jumpTo(messageId: string): void;
  openMedia(target: MediaTarget): void;
  openStamp(stampId: string): void;
  showReactions(message: chat.Message): void;
  showReceipts(message: chat.Message): void;
  /** Opens the camera to answer a POV request. */
  answerPov(requestId: string): void;
  /** Opens the full emoji picker to react to a message. */
  pickReaction(message: chat.Message): void;
}

const ConversationContext = createContext<ConversationContextValue | null>(null);

export const ConversationProvider = ConversationContext.Provider;

export function useConversationContext(): ConversationContextValue {
  const value = useContext(ConversationContext);
  if (!value) throw new Error("useConversationContext outside a conversation screen");
  return value;
}
