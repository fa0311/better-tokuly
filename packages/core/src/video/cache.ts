import type { QueryClient } from "@tanstack/query-core";
import type { video } from "@tokuly/api";
import { videoKeys } from "./keys";

type SummaryPatch = (summary: video.VideoSummary) => video.VideoSummary | null;

const REMOVED = Symbol("removed");

function isSummary(value: object): value is video.VideoSummary {
  return (value as { $typeName?: string }).$typeName === "tokuly.video.v1.VideoSummary";
}

/**
 * Rebuilds a cached value with `patch` applied to every video card in it
 * (`null` drops the card from the list that holds it). Branches without a
 * change keep their identity, so only what changed re-renders.
 */
function patchSummaries(value: unknown, patch: SummaryPatch): unknown {
  if (Array.isArray(value)) {
    let changed = false;
    const next: unknown[] = [];
    for (const item of value) {
      const mapped = patchSummaries(item, patch);
      if (mapped !== item) changed = true;
      // A list row that only wraps a removed card (history items, playlist items) goes with it.
      if (mapped === REMOVED) continue;
      next.push(mapped);
    }
    return changed ? next : value;
  }
  if (value === null || typeof value !== "object" || value instanceof Uint8Array) return value;
  if (isSummary(value)) return patch(value) ?? REMOVED;
  let next: Record<string, unknown> | null = null;
  for (const [key, item] of Object.entries(value)) {
    const mapped = patchSummaries(item, patch);
    if (mapped === item) continue;
    // `{ video: <removed> }` is a row about that video: remove the row.
    if (mapped === REMOVED && key === "video") return REMOVED;
    next ??= { ...(value as Record<string, unknown>) };
    next[key] = mapped === REMOVED ? undefined : mapped;
  }
  return next ?? value;
}

/** Applies `patch` to every cached card of the video, wherever it is listed. */
export function patchVideoEverywhere(
  queryClient: QueryClient,
  videoId: string,
  patch: SummaryPatch,
): void {
  queryClient.setQueriesData({ queryKey: videoKeys.all }, (data: unknown) => {
    if (data === undefined) return data;
    const next = patchSummaries(data, (summary) =>
      summary.id === videoId || summary.shortId === videoId ? patch(summary) : summary,
    );
    // A cached value that is itself the card is not a list to drop it from.
    return next === REMOVED ? data : next;
  });
}

/** Drops a video from every cached list (after deleting it, or "not interested"). */
export function removeVideoEverywhere(queryClient: QueryClient, videoId: string): void {
  patchVideoEverywhere(queryClient, videoId, () => null);
}
