import { useQuery } from "@tanstack/react-query";
import { emoji } from "@tokuly/api";
import { api } from "#/lib/api";

// Typed clients of the emoji domain, one per service.
export const emojiClient = () => api().service(emoji.EmojiService);
export const submissionClient = () => api().service(emoji.EmojiSubmissionService);
export const emojiAdminClient = () => api().service(emoji.EmojiAdminService);

export const emojiKeys = {
  all: ["emoji"] as const,
  deck: ["emoji", "deck"] as const,
  recent: ["emoji", "recent"] as const,
  popular: ["emoji", "popular"] as const,
  newest: ["emoji", "newest"] as const,
  search: (query: string, category: string | undefined) =>
    ["emoji", "search", query, category ?? null] as const,
  author: (userId: string) => ["emoji", "author", userId] as const,
  details: ["emoji", "detail"] as const,
  detail: (nameOrId: string) => ["emoji", "detail", nameOrId] as const,
  submitter: ["emoji", "submitter"] as const,
  submissions: ["emoji", "submissions"] as const,
  submissionList: (state: emoji.EmojiState) => ["emoji", "submissions", state] as const,
  nameCheck: (name: string) => ["emoji", "name", name] as const,
  admin: ["emoji", "admin"] as const,
  adminQueue: ["emoji", "admin", "queue"] as const,
  adminList: (filters: object) => ["emoji", "admin", "list", filters] as const,
  adminTrusted: ["emoji", "admin", "trusted"] as const,
  adminImports: ["emoji", "admin", "imports"] as const,
  adminImport: (id: string) => ["emoji", "admin", "import", id] as const,
  adminImportRest: (id: string, cursor: string) =>
    ["emoji", "admin", "import-rest", id, cursor] as const,
  adminExports: ["emoji", "admin", "exports"] as const,
};

/** The caller's reaction deck: the emoji they pinned, in their order (the server's default until they save one). */
export function useDeck() {
  return useQuery({
    queryKey: emojiKeys.deck,
    queryFn: () => emojiClient().getDeck({}),
    staleTime: 5 * 60_000,
  });
}

/**
 * The emoji the caller used last, most recent first. Read again whenever a
 * screen that shows them opens: other features record uses on the server
 * (a message sent, a reaction given) without this cache hearing of it.
 */
export function useRecentEmoji() {
  return useQuery({
    queryKey: emojiKeys.recent,
    queryFn: async () => (await emojiClient().listRecentEmoji({})).items,
    refetchOnMount: "always",
  });
}

/** One emoji with author, licence and review state; `nameOrId` may carry colons. */
export function useEmojiDetail(nameOrId: string) {
  return useQuery({
    queryKey: emojiKeys.detail(nameOrId),
    queryFn: async () => (await emojiClient().getEmoji({ emoji: nameOrId })).emoji ?? null,
  });
}

/** How an emoji is written in text, decks and `RecordUse`. */
export const shortcode = (name: string) => `:${name}:`;

/** The name inside a `:shortcode:`, or `undefined` for anything else (a Unicode emoji). */
export function shortcodeName(text: string): string | undefined {
  return /^:[a-z0-9_]{2,32}:$/i.test(text) ? text.slice(1, -1).toLowerCase() : undefined;
}

/**
 * The page of an emoji. The server links to `/emoji/<name>`; the one name
 * that is also a page of this area is written with its colons, which
 * `GetEmoji` accepts.
 */
export function emojiPageParam(name: string): string {
  return name === "submissions" ? shortcode(name) : name;
}
