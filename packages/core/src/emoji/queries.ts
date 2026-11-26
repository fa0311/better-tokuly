import type { emoji } from "@tokuly/api";
import type { ClientSource } from "../query";
import { emojiKeys } from "./keys";

type Emoji = ClientSource<typeof emoji.EmojiService>;

/** The most emoji a deck holds. */
export const DECK_MAX = 100;

/** Query options of the emoji domain. Each app adds `enabled` and its hooks around them. */
export const emojiQueries = {
  /** The caller's reaction deck: the emoji they pinned, in their order (the server's default until they save one). */
  deck: (client: Emoji) => ({
    queryKey: emojiKeys.deck,
    queryFn: () => client().getDeck({}),
    staleTime: 5 * 60_000,
  }),

  /**
   * The emoji the caller used last, most recent first. Read again whenever a
   * screen that shows them opens: other features record uses on the server
   * (a message sent, a reaction given) without this cache hearing of it.
   */
  recent: (client: Emoji) => ({
    queryKey: emojiKeys.recent,
    queryFn: async () => (await client().listRecentEmoji({})).items,
    refetchOnMount: "always" as const,
  }),

  /** One emoji with author, licence and review state; `nameOrId` may carry colons. */
  detail: (client: Emoji, nameOrId: string) => ({
    queryKey: emojiKeys.detail(nameOrId),
    queryFn: async () => (await client().getEmoji({ emoji: nameOrId })).emoji ?? null,
  }),
};
