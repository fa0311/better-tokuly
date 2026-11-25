/**
 * The strings of the areas every Tokuly app has (session, map, friends, chat,
 * notifications, stories, shorts, profile and the common words), in Japanese
 * and English, as i18next resources: one namespace per file. An app adds the
 * namespaces of its own screens next to these.
 */
import en from "./en";
import ja from "./ja";

export { en, ja };

export type Namespace = keyof typeof ja;
export const NAMESPACES = Object.keys(ja) as Namespace[];

export const SUPPORTED_LANGUAGES = ["ja", "en"] as const;
export type Language = (typeof SUPPORTED_LANGUAGES)[number];

/** Japanese first: Tokuly's home audience. Anything else falls back to English. */
export function normalizeLanguage(input: string | null | undefined): Language {
  if (!input) return "ja";
  return input.toLowerCase().startsWith("ja") ? "ja" : "en";
}
