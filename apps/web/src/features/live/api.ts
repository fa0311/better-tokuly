import { useQuery } from "@tanstack/react-query";
import { live, social, video } from "@tokuly/api";
import { liveKeys, liveQueries, patchStream, refreshStream, storeStream } from "@tokuly/core/live";
import { api } from "#/lib/api";

export const streamClient = () => api().service(live.LiveStreamService);
export const chatClient = () => api().service(live.LiveChatService);
export const pollClient = () => api().service(live.LivePollService);
export const giftClient = () => api().service(live.LiveGiftService);
export const clipClient = () => api().service(live.ClipService);
export const overlayClient = () => api().service(live.OverlayService);
export const analyticsClient = () => api().service(live.LiveAnalyticsService);
export const videoClient = () => api().service(video.VideoService);

// The keys, query options and cache edits of live live in `@tokuly/core/live`; the folder reads them from here.
export { nextCursor } from "@tokuly/core/query";
export { liveKeys, patchStream, refreshStream, storeStream };

/**
 * A stream page as the caller sees it; `null` when it does not exist (or is
 * not theirs to see). `streamId` is the UUID or the short ID.
 */
export function useStream(streamId: string) {
  return useQuery({ ...liveQueries.stream(streamClient, streamId), enabled: streamId !== "" });
}

/** The gift ladder. It never changes while the app is open. */
export function useGiftTiers() {
  return useQuery(liveQueries.tiers(giftClient));
}

const friendClient = () => api().service(social.FriendService);

/** The IDs of the people the signed-in viewer blocked (pushed chat lines from them are dropped). */
export function useBlockedIds(enabled: boolean) {
  return useQuery({ ...liveQueries.blocked(friendClient), enabled });
}

/** Friends' streams (friends-only ones included): on air first, then the scheduled ones. */
export function useFriendStreams() {
  return useQuery({
    queryKey: liveKeys.friends,
    queryFn: async () => (await streamClient().listFriendStreams({ page: { limit: 20 } })).streams,
    staleTime: 30_000,
  });
}
