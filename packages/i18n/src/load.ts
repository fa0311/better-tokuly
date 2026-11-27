import type { Language } from "./language";

/**
 * The shared namespaces one at a time, for an app that loads only the
 * language it shows and only the areas it opens (the web). Bundlers make
 * one chunk per file of this import.
 */
export const NAMESPACES = [
  "accounts",
  "ask",
  "auth",
  "chat",
  "clips",
  "common",
  "community",
  "emoji",
  "errors",
  "follows",
  "footprints",
  "friends",
  "history",
  "live",
  "map",
  "mobile",
  "notelists",
  "notes",
  "notifications",
  "places",
  "plans",
  "profile",
  "quest",
  "school",
  "search",
  "settings",
  "shorts",
  "spaces",
  "stamps",
  "stories",
  "video",
  "wallet",
] as const;
export type Namespace = (typeof NAMESPACES)[number];

export async function loadNamespace(
  language: Language,
  namespace: Namespace,
): Promise<Record<string, unknown>> {
  return (await import(`./${language}/${namespace}.ts`)).default;
}
