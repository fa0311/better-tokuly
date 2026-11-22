/** A synonym group holds 2 to 20 words. */
export const MIN_WORDS = 2;
export const MAX_WORDS = 20;

/** The words of a group as typed: separated by commas, 読点 or line breaks. */
export function parseWords(text: string): string[] {
  const seen = new Set<string>();
  return text
    .split(/[,、\n]+/)
    .map((word) => word.trim())
    .filter((word) => {
      const key = word.toLowerCase();
      if (!word || seen.has(key)) return false;
      seen.add(key);
      return true;
    });
}
