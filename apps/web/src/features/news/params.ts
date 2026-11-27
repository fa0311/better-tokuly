/** What the news list shows, as kept in the address: `/news?order=latest&tag=…&source=…&lang=…`. */
export interface NewsParams {
  /** Absent: the recommended order. */
  order?: "latest";
  tag?: string;
  /** Only articles of this source (its ID). */
  source?: string;
  /** Only sources in this language (a BCP 47 tag). */
  lang?: string;
}

/** A parameter as the text that was in the address (the router parses `tag=2026` into a number). */
function text(value: unknown): string | undefined {
  if (typeof value === "string") return value.trim() || undefined;
  return typeof value === "number" || typeof value === "boolean" ? String(value) : undefined;
}

export function parseNewsParams(input: Record<string, unknown>): NewsParams {
  return {
    order: input.order === "latest" ? "latest" : undefined,
    tag: text(input.tag)?.toLowerCase(),
    source: text(input.source),
    lang: text(input.lang),
  };
}
