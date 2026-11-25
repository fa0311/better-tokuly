import type { chat, common } from "@tokuly/api";
import { chatSentences, chatTimes, isGroup, peerOf } from "@tokuly/core/chat";
import i18n from "#/lib/i18n";

/** Chat's sentences and times (`@tokuly/core/chat`) in the app's language. */
export const { previewText, contentText, replyText, systemText } = chatSentences(i18n);
export const { clock, dayLabel, listTime } = chatTimes(i18n);

/** A person's name for headings and rows. */
export function displayName(user: common.UserSummary | undefined): string {
  if (!user || user.deleted) return i18n.t("chat:people.deleted");
  return user.displayName || `@${user.handle}`;
}

/** What a conversation is called: the group's name, or the other person. */
export function titleOf(conversation: chat.Conversation, myId: string): string {
  if (isGroup(conversation)) return conversation.name || i18n.t("chat:group.unnamed");
  return displayName(peerOf(conversation, myId));
}
