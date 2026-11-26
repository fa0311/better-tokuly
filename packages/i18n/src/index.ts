/**
 * The strings of the areas every Tokuly app has (session, map, friends, chat,
 * notifications, stories, shorts, video, notes with their lists and clips, search,
 * live, profile, settings, spaces and the common words), and the phone app's own screens
 * (`mobile`: React Native has no per-app glob to find them), in Japanese
 * and English, as i18next resources: one namespace per file. The web adds the namespaces of its own screens next to these.
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
