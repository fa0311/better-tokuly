import { useMutation, useQueryClient } from "@tanstack/react-query";
import { create, emoji } from "@tokuly/api";
import { toast } from "sonner";
import { errorMessage } from "#/lib/errors";
import { emojiClient, emojiKeys } from "./api";

/** The most emoji a deck holds. */
export const DECK_MAX = 100;

const SAVE_KEY = ["emoji", "deck", "save"] as const;

/**
 * Saves the reaction deck as the given list (`:name:` or a Unicode emoji
 * each, in order). The deck on screen changes at once and goes back if the
 * server refuses.
 */
export function useSaveDeck() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationKey: SAVE_KEY,
    mutationFn: (emojis: string[]) => emojiClient().setDeck({ emojis }),
    onMutate: async (emojis) => {
      await queryClient.cancelQueries({ queryKey: emojiKeys.deck });
      const previous = queryClient.getQueryData<emoji.GetDeckResponse>(emojiKeys.deck);
      // Custom emoji are drawn from the catalogue, so the names are enough until the server answers.
      queryClient.setQueryData(
        emojiKeys.deck,
        create(emoji.GetDeckResponseSchema, {
          items: emojis.map((item) => ({ emoji: item })),
          isDefault: false,
        }),
      );
      return { previous };
    },
    onError: (error, _emojis, context) => {
      queryClient.setQueryData(emojiKeys.deck, context?.previous);
      toast.error(errorMessage(error, { namespace: "emoji" }));
    },
    onSuccess: (response) => {
      // While a later change is on its way, its picture of the deck is the newer one.
      if (queryClient.isMutating({ mutationKey: SAVE_KEY }) > 1) return;
      queryClient.setQueryData(
        emojiKeys.deck,
        create(emoji.GetDeckResponseSchema, { items: response.items, isDefault: false }),
      );
    },
  });
}
