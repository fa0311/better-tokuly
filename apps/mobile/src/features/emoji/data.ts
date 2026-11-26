import { type QueryClient, useQuery } from "@tanstack/react-query";
import { create, emoji } from "@tokuly/api";
import { emojiKeys, emojiQueries } from "@tokuly/core/emoji";
import { currentAccount, isActive } from "#/features/session/account";
import { emojiClient } from "./catalog";

/** Strings of this area (`@tokuly/i18n`). */
export const NS = "emoji";

export function useDeck() {
  return useQuery(emojiQueries.deck(emojiClient));
}

export function useRecentEmoji(enabled = true) {
  return useQuery({ ...emojiQueries.recent(emojiClient), enabled });
}

export function useEmojiDetail(name: string) {
  return useQuery({ ...emojiQueries.detail(emojiClient, name), enabled: name !== "" });
}

/**
 * Saves the deck as `emojis` (`:name:` or a Unicode emoji each, in order),
 * as the account signed in now. The answer is that account's: after a
 * switch it is dropped, its refusal too (README, "Accounts"). `true` when it
 * went through for the person still here.
 */
export async function saveDeck(queryClient: QueryClient, emojis: string[]): Promise<boolean> {
  const account = currentAccount();
  if (!account) return false;
  try {
    const { items } = await account.api.service(emoji.EmojiService).setDeck({ emojis });
    if (!isActive(account)) return false;
    queryClient.setQueryData(
      emojiKeys.deck,
      create(emoji.GetDeckResponseSchema, { items, isDefault: false }),
    );
    return true;
  } catch (error) {
    if (isActive(account)) throw error;
    return false;
  }
}
