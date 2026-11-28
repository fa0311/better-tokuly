import { chat } from "@tokuly/api";

/** Whether `text` mentions `@handle` as a word (not `@handle2`, not `mail@handle`). */
export function mentionsBot(text: string, handle: string): boolean {
  const escaped = handle.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`(^|[^\\w@])@${escaped}(?![\\w-])`, "i").test(text);
}

/**
 * Why Tokuly Bot will not answer what the writer is typing in a group, if
 * it will not: their own bot switch, or their AI assistant, is off. The bot
 * stays silent then, so that nobody else learns about their settings; the
 * writer's own app tells them instead.
 */
export function botHint(
  settings: chat.AssistSettings | undefined,
  text: string,
): "botOff" | "assistOff" | null {
  if (!settings?.bot || !mentionsBot(text, settings.bot.handle)) return null;
  if (!settings.botEnabled) return "botOff";
  if (settings.effectiveProvider === chat.AiProvider.UNSPECIFIED) return "assistOff";
  return null;
}
