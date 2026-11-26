import { useQuery } from "@tanstack/react-query";
import { emoji } from "@tokuly/api";
import { emojiQueries } from "@tokuly/core/emoji";
import { api } from "#/lib/api";

// Typed clients of the emoji domain, one per service.
export const emojiClient = () => api().service(emoji.EmojiService);
export const submissionClient = () => api().service(emoji.EmojiSubmissionService);
export const emojiAdminClient = () => api().service(emoji.EmojiAdminService);

/** The caller's reaction deck: the emoji they pinned, in their order (the server's default until they save one). */
export function useDeck() {
  return useQuery(emojiQueries.deck(emojiClient));
}

/** The emoji the caller used last, most recent first, read again whenever a screen that shows them opens. */
export function useRecentEmoji() {
  return useQuery(emojiQueries.recent(emojiClient));
}

/** One emoji with author, licence and review state; `nameOrId` may carry colons. */
export function useEmojiDetail(nameOrId: string) {
  return useQuery(emojiQueries.detail(emojiClient, nameOrId));
}
