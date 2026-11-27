import type { chat } from "@tokuly/api";

export const CONSENT_KEY = "tokuly.chat.assistConsent";

/**
 * Where the agreement to the assistant's terms is remembered: per provider
 * and per actual recipient (the company, or the host its base URL points
 * at), so that a change of either asks again. Empty when nothing would
 * receive the text.
 */
export function consentStorageKey(
  settings: Pick<chat.AssistSettings, "effectiveProvider" | "effectiveRecipient"> | undefined,
): string {
  if (!settings?.effectiveProvider || !settings.effectiveRecipient) return "";
  return `${CONSENT_KEY}:${settings.effectiveProvider}:${settings.effectiveRecipient.toLowerCase()}`;
}
