import { type QueryClient, useQuery } from "@tanstack/react-query";
import { video } from "@tokuly/api";
import { api } from "#/lib/api";

export const channelClient = () => api().service(video.ChannelService);
export const videoClient = () => api().service(video.VideoService);
export const playbackClient = () => api().service(video.PlaybackService);
export const playlistClient = () => api().service(video.PlaylistService);
export const feedClient = () => api().service(video.VideoFeedService);
export const partyClient = () => api().service(video.WatchPartyService);
export const analyticsClient = () => api().service(video.StudioAnalyticsService);

/** Query keys of the video domain. Everything starts with `"video"`, so one invalidation reaches all of it. */
export const videoKeys = {
  all: ["video"] as const,
  feeds: ["video", "feed"] as const,
  home: ["video", "feed", "home"] as const,
  subscriptionsFeed: ["video", "feed", "subscriptions"] as const,
  trending: (category: video.VideoCategory) => ["video", "feed", "trending", category] as const,
  related: (videoId: string) => ["video", "feed", "related", videoId] as const,
  video: (videoId: string) => ["video", "video", videoId] as const,
  videos: ["video", "video"] as const,
  channels: ["video", "channel"] as const,
  channel: (handle: string) => ["video", "channel", "page", handle] as const,
  channelVideos: (handle: string, sort: video.ChannelVideoSort, origin: video.VideoOrigin) =>
    ["video", "channel", "videos", handle, sort, origin] as const,
  channelPlaylists: (handle: string) => ["video", "channel", "playlists", handle] as const,
  subscribers: (channelId: string) => ["video", "channel", "subscribers", channelId] as const,
  myChannels: ["video", "channel", "mine"] as const,
  subscriptions: ["video", "channel", "subscriptions"] as const,
  /** Outside `channels` on purpose: subscribing to a suggestion must not make the list jump. */
  recommendedChannels: ["video", "recommended-channels"] as const,
  playlists: ["video", "playlist"] as const,
  myPlaylists: (containsVideoId = "") => ["video", "playlist", "mine", containsVideoId] as const,
  playlist: (playlistId: string) => ["video", "playlist", "one", playlistId] as const,
  playlistItems: (playlistId: string) => ["video", "playlist", "items", playlistId] as const,
  history: ["video", "history"] as const,
  studio: ["video", "studio"] as const,
  studioVideos: (channelId: string, status: video.VideoStatus) =>
    ["video", "studio", "videos", channelId, status] as const,
  games: (query: string) => ["video", "studio", "games", query] as const,
  subtitles: (videoId: string) => ["video", "studio", "subtitles", videoId] as const,
  channelAnalytics: (channelId: string, days: number) =>
    ["video", "studio", "analytics", "channel", channelId, days] as const,
  videoAnalytics: (videoId: string, days: number) =>
    ["video", "studio", "analytics", "video", videoId, days] as const,
  realtimeAnalytics: (kind: "channel" | "video", id: string) =>
    ["video", "studio", "analytics", "realtime", kind, id] as const,
  party: (code: string) => ["video", "party", code] as const,
  partyPreview: (code: string) => ["video", "party", "preview", code] as const,
};

/** The signed-in person's channels (default first) and how many they may have. */
export function useMyChannels() {
  return useQuery({
    queryKey: videoKeys.myChannels,
    queryFn: () => channelClient().listMyChannels({}),
    staleTime: 60_000,
  });
}

/** A video as the watch page and the studio editor show it. `videoId` is the UUID or the short ID. */
export function useVideo(videoId: string) {
  return useQuery({
    queryKey: videoKeys.video(videoId),
    queryFn: async () => (await videoClient().getVideo({ videoId })).video!,
  });
}

// ───────────────────────────── Cache patching ─────────────────────────────

type SummaryPatch = (summary: video.VideoSummary) => video.VideoSummary | null;

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

const REMOVED = Symbol("removed");

/** Applies `patch` to every cached card of the video, wherever it is listed. */
export function patchVideoEverywhere(
  queryClient: QueryClient,
  videoId: string,
  patch: (summary: video.VideoSummary) => video.VideoSummary | null,
): void {
  queryClient.setQueriesData({ queryKey: videoKeys.all }, (data: unknown) =>
    data === undefined
      ? data
      : patchSummaries(data, (summary) =>
          summary.id === videoId || summary.shortId === videoId ? patch(summary) : summary,
        ),
  );
}

/** Drops a video from every cached list (after deleting it, or "not interested"). */
export function removeVideoEverywhere(queryClient: QueryClient, videoId: string): void {
  patchVideoEverywhere(queryClient, videoId, () => null);
}

/** `getNextPageParam` for the API's cursor pages. */
export function nextCursor(last: { page?: { nextCursor: string } }): string | undefined {
  return last.page?.nextCursor || undefined;
}
