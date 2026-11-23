import { type QueryClient, useQuery } from "@tanstack/react-query";
import { shorts } from "@tokuly/api";
import { api } from "#/lib/api";

export const shortClient = () => api().service(shorts.ShortService);
export const feedClient = () => api().service(shorts.ShortFeedService);
export const discoveryClient = () => api().service(shorts.ShortDiscoveryService);
export const analyticsClient = () => api().service(shorts.ShortAnalyticsService);

/** Query keys of the shorts domain. Everything starts with `"shorts"`, so one invalidation reaches all of it. */
export const shortKeys = {
  all: ["shorts"] as const,
  short: (shortId: string) => ["shorts", "short", shortId] as const,
  feeds: ["shorts", "feed"] as const,
  lists: ["shorts", "list"] as const,
  mine: (channelId: string, status: shorts.ShortStatus) =>
    ["shorts", "list", "mine", channelId, status] as const,
  hashtag: (tag: string) => ["shorts", "hashtag", tag] as const,
  trendingHashtags: ["shorts", "hashtag", "trending"] as const,
  hashtagSuggestions: (prefix: string) => ["shorts", "hashtag", "suggest", prefix] as const,
  sound: (soundId: string) => ["shorts", "sound", soundId] as const,
  soundSuggestions: (query: string) => ["shorts", "sound", "suggest", query] as const,
  analytics: (kind: "short" | "channel", id: string, start: string, end: string) =>
    ["shorts", "analytics", kind, id, start, end] as const,
};

/** One short as the viewer may see it. `shortId` is the UUID or the 11-character ID. */
export function useShort(shortId: string | undefined) {
  return useQuery({
    queryKey: shortKeys.short(shortId ?? ""),
    queryFn: async () => (await shortClient().getShort({ shortId })).short!,
    enabled: Boolean(shortId),
    retry: false,
  });
}

/** `getNextPageParam` for the API's cursor pages. */
export function nextCursor(last: { page?: { nextCursor: string } }): string | undefined {
  return last.page?.nextCursor || undefined;
}

// ───────────────────────────── Cache patching ─────────────────────────────

const REMOVED = Symbol("removed");

function isShort(value: object): value is shorts.Short {
  return (value as { $typeName?: string }).$typeName === "tokuly.shorts.v1.Short";
}

/**
 * Rebuilds a cached value with `patch` applied to every short in it (`null`
 * drops the short, and a feed item wrapping it, from the list that holds
 * it). Branches without a change keep their identity.
 */
function patchShorts(value: unknown, patch: (short: shorts.Short) => shorts.Short | null): unknown {
  if (Array.isArray(value)) {
    let changed = false;
    const next: unknown[] = [];
    for (const item of value) {
      const mapped = patchShorts(item, patch);
      if (mapped !== item) changed = true;
      if (mapped !== REMOVED) next.push(mapped);
    }
    return changed ? next : value;
  }
  if (value === null || typeof value !== "object" || value instanceof Uint8Array) return value;
  if (isShort(value)) return patch(value) ?? REMOVED;
  let next: Record<string, unknown> | null = null;
  for (const [key, item] of Object.entries(value)) {
    const mapped = patchShorts(item, patch);
    if (mapped === item) continue;
    // `{ short: <removed> }` is a row about that short (a feed item, a top-shorts row): it goes too.
    if (mapped === REMOVED && key === "short") return REMOVED;
    next ??= { ...(value as Record<string, unknown>) };
    next[key] = mapped === REMOVED ? undefined : mapped;
  }
  return next ?? value;
}

/** Applies `patch` to every cached copy of the short: feeds, grids, the studio list, its own entry. */
export function patchShortEverywhere(
  queryClient: QueryClient,
  shortId: string,
  patch: (short: shorts.Short) => shorts.Short | null,
): void {
  queryClient.setQueriesData({ queryKey: shortKeys.all }, (data: unknown) => {
    if (data === undefined) return data;
    const next = patchShorts(data, (short) =>
      short.id === shortId || short.shortId === shortId ? patch(short) : short,
    );
    // The short's own entry is not a list to drop it from.
    return next === REMOVED ? data : next;
  });
}

/** Drops a short from every cached list (after deleting it, or "not interested"). */
export function removeShortEverywhere(queryClient: QueryClient, shortId: string): void {
  patchShortEverywhere(queryClient, shortId, () => null);
}

/** Changes the viewer's own state on a short (saved, subscribed, …) wherever it is cached. */
export function patchViewer(
  queryClient: QueryClient,
  shortId: string,
  viewer: Partial<Pick<shorts.ShortViewerState, "saved" | "subscribed" | "notInterested">>,
  fields: Partial<Pick<shorts.Short, "saveCount" | "shareCount" | "pinned">> = {},
): void {
  patchShortEverywhere(queryClient, shortId, (short) => ({
    ...short,
    ...fields,
    viewer: short.viewer ? { ...short.viewer, ...viewer } : short.viewer,
  }));
}
