import { useQueryClient } from "@tanstack/react-query";
import type { chat } from "@tokuly/api";
import { toast } from "sonner";
import { errorMessage } from "#/lib/errors";
import { chatKeys, conversationClient } from "./api";
import { resolveUserId } from "./people";

/**
 * Opens (creating it when the two never talked) the direct conversation with
 * a person, given their handle or user ID. Resolves to the conversation, or
 * to `undefined` after telling the viewer why it cannot be opened (unknown
 * person, their privacy settings, a block).
 */
export function useOpenDirect(): (handleOrUserId: string) => Promise<chat.Conversation | undefined> {
  const queryClient = useQueryClient();
  return async (handleOrUserId) => {
    try {
      const userId = await resolveUserId(handleOrUserId);
      const { conversation } = await conversationClient().openDirect({ userId });
      if (!conversation) return undefined;
      // Enough to draw the screen at once; marked old so that opening it still asks for the extras.
      queryClient.setQueryData<chat.Conversation>(chatKeys.conversation(conversation.id), conversation, { updatedAt: 0 });
      return conversation;
    } catch (error) {
      toast.error(errorMessage(error));
      return undefined;
    }
  };
}
