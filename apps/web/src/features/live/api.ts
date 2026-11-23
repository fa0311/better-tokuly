import { type QueryClient, useQuery } from "@tanstack/react-query";
import { errorReason, live, social, video } from "@tokuly/api";
import { api } from "#/lib/api";

export const streamClient = () => api().service(live.LiveStreamService);
export const chatClient = () => api().service(live.LiveChatService);
export const pollClient = () => api().service(live.LivePollService);
export const giftClient = () => api().service(live.LiveGiftService);
export const clipClient = () => api().service(live.ClipService);
export const overlayClient = () => api().service(live.OverlayService);
export const analyticsClient = () => api().service(live.LiveAnalyticsService);
export const videoClient = () => api().service(video.VideoService);

/** Query keys of the live domain. Everything starts with `"live"`. */
export const liveKeys = {
  all: ["live"] as const,
  /** Every list of stream cards (browse, channel, studio). */
  lists: ["live", "list"] as const,
  liveNow: (category: video.VideoCategory, game: string) =>
    ["live", "list", "now", category, game] as const,
  upcoming: (category: video.VideoCategory) => ["live", "list", "upcoming", category] as const,
  friends: ["live", "list", "friends"] as const,
  subscribed: ["live", "list", "subscribed"] as const,
  channelStreams: (channelId: string, status: live.StreamStatus) =>
    ["live", "list", "channel", channelId, status] as const,
  stream: (streamId: string) => ["live", "stream", streamId] as const,
  streams: ["live", "stream"] as const,
  chat: (streamId: string) => ["live", "chat", streamId] as const,
  polls: (streamId: string) => ["live", "polls", streamId] as const,
  viewers: (streamId: string) => ["live", "viewers", streamId] as const,
  supporters: (streamId: string) => ["live", "supporters", streamId] as const,
  tiers: ["live", "tiers"] as const,
  gifts: ["live", "gifts"] as const,
  sentGifts: ["live", "gifts", "sent"] as const,
  receivedGifts: (channelId: string, streamId: string) =>
    ["live", "gifts", "received", channelId, streamId] as const,
  clips: ["live", "clip"] as const,
  clip: (clipId: string) => ["live", "clip", "one", clipId] as const,
  clipList: (kind: "stream" | "video" | "channel" | "creator", id: string) =>
    ["live", "clip", "list", kind, id] as const,
  archive: (videoId: string) => ["live", "archive", videoId] as const,
  realtimeStats: (streamId: string) => ["live", "stats", "realtime", streamId] as const,
  analytics: (streamId: string) => ["live", "stats", "report", streamId] as const,
  reports: (channelId: string) => ["live", "stats", "reports", channelId] as const,
  /** IDs of the people the viewer blocked (pushed chat lines from them are dropped). */
  blocked: ["live", "blocked"] as const,
  overlays: (channelId: string) => ["live", "overlays", channelId] as const,
  presets: ["live", "overlays", "presets"] as const,
  moderation: (channelId: string) => ["live", "moderation", channelId] as const,
  moderators: (channelId: string) => ["live", "moderation", channelId, "moderators"] as const,
  restrictions: (channelId: string) => ["live", "moderation", channelId, "restrictions"] as const,
  bannedWords: (channelId: string) => ["live", "moderation", channelId, "words"] as const,
};

/** `getNextPageParam` for the API's cursor pages. */
export function nextCursor(last: { page?: { nextCursor: string } }): string | undefined {
  return last.page?.nextCursor || undefined;
}

/**
 * A stream page as the caller sees it; `null` when it does not exist (or is
 * not theirs to see). `streamId` is the UUID or the short ID.
 */
export function useStream(streamId: string) {
  return useQuery({
    queryKey: liveKeys.stream(streamId),
    queryFn: async () => {
      try {
        return (await streamClient().getStream({ streamId })).stream ?? null;
      } catch (error) {
        if (errorReason(error) === "STREAM_NOT_FOUND") return null;
        throw error;
      }
    },
    enabled: streamId !== "",
  });
}

/** Changes the cached page of a stream, under whichever ID it was opened with. */
export function patchStream(
  queryClient: QueryClient,
  streamId: string,
  patch: (stream: live.Stream) => live.Stream,
): void {
  for (const [key, stream] of queryClient.getQueriesData<live.Stream | null>({
    queryKey: liveKeys.streams,
  })) {
    if (!stream?.summary) continue;
    if (stream.summary.id !== streamId && stream.summary.shortId !== streamId) continue;
    queryClient.setQueryData(key, patch(stream));
  }
}

/** Puts an answer of the server (create, update, start, end) into the cache under both IDs. */
export function storeStream(queryClient: QueryClient, stream: live.Stream | undefined): void {
  if (!stream?.summary) return;
  queryClient.setQueryData(liveKeys.stream(stream.summary.id), stream);
  queryClient.setQueryData(liveKeys.stream(stream.summary.shortId), stream);
  void queryClient.invalidateQueries({ queryKey: liveKeys.lists });
}

/** The gift ladder. It never changes while the app is open. */
export function useGiftTiers() {
  return useQuery({
    queryKey: liveKeys.tiers,
    queryFn: () => giftClient().getGiftTiers({}),
    staleTime: Number.POSITIVE_INFINITY,
  });
}

/** Reads a stream's page again, under whichever ID it was opened with. */
export function refreshStream(queryClient: QueryClient, streamId: string): void {
  void queryClient.invalidateQueries({
    queryKey: liveKeys.streams,
    predicate: (query) => {
      const stream = query.state.data as live.Stream | null | undefined;
      return (
        query.queryKey[2] === streamId ||
        stream?.summary?.id === streamId ||
        stream?.summary?.shortId === streamId
      );
    },
  });
}

/**
 * The IDs of the people the signed-in viewer blocked. Chat history comes
 * filtered from the server, and so do pushed lines; the room still drops
 * pushed lines from these authors, as a second line of defence.
 */
export async function fetchBlockedIds(): Promise<Set<string>> {
  const ids = new Set<string>();
  let cursor = "";
  // A handful of pages is plenty; a viewer with more blocks than that still gets the newest ones.
  for (let page = 0; page < 5; page++) {
    const response = await api()
      .service(social.FriendService)
      .listBlockedUsers({ page: { cursor, limit: 100 } });
    for (const user of response.users) ids.add(user.id);
    cursor = response.page?.nextCursor ?? "";
    if (!cursor) break;
  }
  return ids;
}

export function useBlockedIds(enabled: boolean) {
  return useQuery({
    queryKey: liveKeys.blocked,
    queryFn: fetchBlockedIds,
    enabled,
    staleTime: 5 * 60_000,
  });
}
