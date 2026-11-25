import type { chat } from "@tokuly/api";
import type { ClientSource } from "../query";
import { chatKeys } from "./keys";

/** What the server allows when it has not told us yet (its own defaults). */
export const DEFAULT_CAPABILITIES = {
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

/** Limits and optional features of this server, as the server described them (rarely changes). */
export function capabilitiesQuery(client: ClientSource<typeof chat.ConversationService>) {
  return {
    queryKey: chatKeys.capabilities,
    queryFn: async () => (await client().getCapabilities({})).capabilities,
    staleTime: 10 * 60_000,
  };
}

/**
 * The limits to work with: the server's where it set one, its defaults
 * otherwise. Until the answer is in, the optional features (calls, AI) count
 * as off, so their controls never flash.
 */
export function capabilitiesOf(data: chat.Capabilities | undefined): Capabilities {
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
