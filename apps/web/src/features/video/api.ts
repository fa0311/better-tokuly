import { useQuery } from "@tanstack/react-query";
import { video } from "@tokuly/api";
import { videoKeys } from "@tokuly/core/video";
import { api } from "#/lib/api";

/** Keys, cache edits and paging are `@tokuly/core/video`; this file wires them to the web's API client. */
export { nextCursor } from "@tokuly/core/query";
export { patchVideoEverywhere, removeVideoEverywhere, videoKeys } from "@tokuly/core/video";

export const channelClient = () => api().service(video.ChannelService);
export const videoClient = () => api().service(video.VideoService);
export const playbackClient = () => api().service(video.PlaybackService);
export const playlistClient = () => api().service(video.PlaylistService);
export const feedClient = () => api().service(video.VideoFeedService);
export const partyClient = () => api().service(video.WatchPartyService);
export const analyticsClient = () => api().service(video.StudioAnalyticsService);

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
