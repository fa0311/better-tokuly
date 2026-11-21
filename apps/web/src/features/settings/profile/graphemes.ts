const segmenter = typeof Intl.Segmenter === "function" ? new Intl.Segmenter(undefined, { granularity: "grapheme" }) : null;

/**
 * Splits text into what a person sees as characters (an emoji with its
 * modifiers is one), which is how the API counts name, bio and status lengths.
 */
export function graphemes(text: string): string[] {
  return segmenter ? Array.from(segmenter.segment(text), (part) => part.segment) : Array.from(text);
}
