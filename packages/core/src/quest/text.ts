import type { common } from "@tokuly/api";

/** A text the server sends in both languages, in the one the app is shown in. */
export function localized(
  text: Pick<common.LocalizedText, "ja" | "en"> | undefined,
  language: string,
): string {
  if (!text) return "";
  return (language.startsWith("ja") ? text.ja || text.en : text.en || text.ja) ?? "";
}
