import type { emoji } from "@tokuly/api";

/** Query keys of the emoji domain. Everything starts with `"emoji"`. */
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
