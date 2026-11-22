import { feed } from "@tokuly/api";

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

/** The `ListNews` request for one page of a list. */
export function listRequest(params: NewsParams, cursor: string, limit: number) {
  return {
    page: { cursor, limit },
    order: params.order === "latest" ? feed.NewsOrder.LATEST : feed.NewsOrder.RECOMMENDED,
    tag: params.tag ?? "",
    sourceId: params.source ?? "",
    language: params.lang ?? "",
  };
}

/** The feedback a press on one of the two buttons leads to. */
export function toggledFeedback(
  current: feed.NewsFeedback,
  pressed: feed.NewsFeedback,
): feed.NewsFeedback {
  return current === pressed ? feed.NewsFeedback.UNSPECIFIED : pressed;
}
