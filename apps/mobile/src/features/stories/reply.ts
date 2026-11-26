import type { QueryClient } from "@tanstack/react-query";
import type { notes } from "@tokuly/api";
import { chatKeys } from "@tokuly/core/chat";
import { conversationClient } from "#/features/chat/data";
import { send } from "#/features/chat/outbox";
import { AccountChanged, currentAccount, ensureActive } from "#/features/session/account";
import i18n from "#/lib/i18n";

/**
 * A reply to a story is an ordinary direct message to its owner: it goes
 * through the chat's outbox, saying which story it answers.
 */
export async function replyInChat(
  queryClient: QueryClient,
  ownerId: string,
  story: notes.Story,
  text: string,
): Promise<void> {
  const account = currentAccount();
  if (!account) throw new AccountChanged();
  const conversations = conversationClient();
  const { conversation } = await conversations.openDirect({ userId: ownerId });
  // The conversation is the replying account's: it is only written to while that account is in use.
  ensureActive(account);
  if (!conversation) return;
  queryClient.setQueryData(chatKeys.conversation(conversation.id), conversation, { updatedAt: 0 });
  send(queryClient, conversation.id, {
    text: story.title
      ? i18n.t("mobile:stories.replyToTitled", { title: story.title, text })
      : i18n.t("mobile:stories.replyTo", { text }),
  });
}
