export const SUPPORTED_LANGUAGES = ["ja", "en"] as const;
export type Language = (typeof SUPPORTED_LANGUAGES)[number];

/** Japanese first: Tokuly's home audience. Anything else falls back to English. */
export function normalizeLanguage(input: string | null | undefined): Language {
  if (!input) return "ja";
  return input.toLowerCase().startsWith("ja") ? "ja" : "en";
}
