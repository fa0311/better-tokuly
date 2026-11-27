import { feed } from "@tokuly/api";
import type { NewsParams } from "./params";

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
