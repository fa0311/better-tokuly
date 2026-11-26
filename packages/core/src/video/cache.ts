import type { QueryClient } from "@tanstack/query-core";
import type { video } from "@tokuly/api";
import { videoKeys } from "./keys";
import { withRating } from "./watch";

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

/**
 * Rates the video of a watch page (cached under `videoId`): the thumb and
 * its count move at once, the server's answer (`send`) replaces them, and a
 * refusal puts back what they were before it is thrown on.
 *
 * A read of the page that is on its way (a refetch on focus, after an
 * invalidation) may have been answered before the rating: landing later, it
 * would put the old thumb and count back. Such a read is dropped, when the
 * thumb moves and again when the answer is in.
 */
export async function rateVideo(
  queryClient: QueryClient,
  videoId: string,
  next: video.Rating,
  send: () => Promise<{ rating: video.Rating; likeCount: bigint }>,
): Promise<void> {
  const key = videoKeys.video(videoId);
  const dropReads = () => queryClient.cancelQueries({ queryKey: key, exact: true });
  const edit = (change: (current: video.Video) => video.Video) =>
    queryClient.setQueryData<video.Video | null>(key, (current) =>
      current ? change(current) : current,
    );

  await dropReads();
  const before = queryClient.getQueryData<video.Video | null>(key);
  edit((current) => withRating(current, next));
  try {
    const answer = await send();
    await dropReads();
    edit((current) => rated(current, answer.rating, answer.likeCount));
  } catch (error) {
    // Only the thumb goes back: whatever else changed meanwhile (views counted) stays.
    if (before?.viewer && before.summary) {
      const { rating } = before.viewer;
      const { likeCount } = before.summary;
      edit((current) => rated(current, rating, likeCount));
    }
    throw error;
  }
}

function rated(item: video.Video, rating: video.Rating, likeCount: bigint): video.Video {
  return {
    ...item,
    viewer: item.viewer && { ...item.viewer, rating },
    summary: item.summary && { ...item.summary, likeCount },
  };
}
