import { useQueryClient } from "@tanstack/react-query";
import { type chat, fromDate } from "@tokuly/api";
import { toast } from "sonner";
import { useTranslation } from "#/lib/use-translation";
import { conversationClient } from "./api";
import { dropConversation, putAnswer, refreshUnread } from "./conversations";
import { errorMessage } from "./errors";
import { markRead } from "./messages";

/** How long a mute lasts; `null` = until the person unmutes. */
export type MuteFor = number | null;

/**
 * What a member can do with a conversation as a whole: their preferences
 * (pin, mute, archive), hiding, leaving, answering a message request. Each
 * action updates what the client holds and reports failures with a toast;
 * the promise resolves to whether it worked.
 */
export function useConversationActions() {
  const queryClient = useQueryClient();
  const { t } = useTranslation("chat");

  const attempt = async (work: () => Promise<void>, done?: string): Promise<boolean> => {
    try {
      await work();
      if (done) toast.success(done);
      return true;
    } catch (error) {
      toast.error(errorMessage(error));
      return false;
    }
  };

  const update = (
    conversation: chat.Conversation,
    change: { pinned?: boolean; muted?: boolean; mutedUntil?: Date; archived?: boolean },
    done?: string,
  ) =>
    attempt(async () => {
      await putAnswer(queryClient, conversation.id, () =>
        conversationClient().updatePreferences({
          conversationId: conversation.id,
          pinned: change.pinned,
          muted: change.muted,
          mutedUntil: change.mutedUntil ? fromDate(change.mutedUntil) : undefined,
          archived: change.archived,
        }),
      );
      // Muting and archiving move messages in and out of the totals.
      if (change.muted !== undefined || change.archived !== undefined) refreshUnread(queryClient);
    }, done);

  return {
    setPinned: (conversation: chat.Conversation, pinned: boolean) =>
      update(conversation, { pinned }, pinned ? t("toast.pinned") : t("toast.unpinned")),
    mute: (conversation: chat.Conversation, forMs: MuteFor) =>
      update(
        conversation,
        { muted: true, mutedUntil: forMs === null ? undefined : new Date(Date.now() + forMs) },
        t("toast.muted"),
      ),
    unmute: (conversation: chat.Conversation) =>
      update(conversation, { muted: false }, t("toast.unmuted")),
    setArchived: (conversation: chat.Conversation, archived: boolean) =>
      update(conversation, { archived }, archived ? t("toast.archived") : t("toast.unarchived")),
    markRead: (conversation: chat.Conversation) =>
      attempt(() => markRead(queryClient, conversation.id)),
    hide: (conversation: chat.Conversation, clearHistory: boolean) =>
      attempt(
        async () => {
          await conversationClient().hideConversation({
            conversationId: conversation.id,
            clearHistory,
          });
          dropConversation(queryClient, conversation.id);
          refreshUnread(queryClient);
        },
        clearHistory ? t("toast.cleared") : t("toast.hidden"),
      ),
    leave: (conversation: chat.Conversation) =>
      attempt(async () => {
        await conversationClient().leaveConversation({ conversationId: conversation.id });
        dropConversation(queryClient, conversation.id);
        refreshUnread(queryClient);
      }, t("toast.left")),
    /** Answers a message request. Declining is silent for the sender. */
    respond: (conversation: chat.Conversation, accept: boolean) =>
      attempt(
        async () => {
          const respond = () =>
            conversationClient().respondToRequest({ conversationId: conversation.id, accept });
          if (accept) await putAnswer(queryClient, conversation.id, respond);
          else {
            await respond();
            dropConversation(queryClient, conversation.id);
          }
          refreshUnread(queryClient);
        },
        accept ? t("toast.requestAccepted") : t("toast.requestDeclined"),
      ),
  };
}
