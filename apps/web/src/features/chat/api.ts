import { useQuery } from "@tanstack/react-query";
import { chat } from "@tokuly/api";
import { type Capabilities, capabilitiesOf, capabilitiesQuery } from "@tokuly/core/chat";
import { api } from "#/lib/api";

// The chat area's keys live in `@tokuly/core/chat`; most of the folder reads them from here.
export { type Capabilities, chatKeys, LIST_FILTERS, type ListFilter } from "@tokuly/core/chat";

export const conversationClient = () => api().service(chat.ConversationService);
export const messageClient = () => api().service(chat.MessageService);
export const povClient = () => api().service(chat.PovService);
export const callClient = () => api().service(chat.CallService);
export const assistClient = () => api().service(chat.AssistService);

/**
 * Limits and optional features of this server. Until the answer is in, the
 * optional features (calls, AI) count as off, so their controls never flash.
 */
export function useCapabilities(): Capabilities {
  const { data } = useQuery(capabilitiesQuery(conversationClient));
  return capabilitiesOf(data);
}
