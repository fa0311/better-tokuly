import {
  type QueryClient,
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { common, video } from "@tokuly/api";
import { nextCursor } from "@tokuly/core/query";
import {
  type Feed,
  feedKey,
  loadFeedPage,
  nextInQueue,
  patchVideoEverywhere,
  videoKeys,
  withRating,
} from "@tokuly/core/video";
import { Alert } from "react-native";
import { api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";

/**
 * Videos in the query cache. Keys, pages, cache edits and the watch page's
 * rules are `@tokuly/core/video`; this file wires them to the phone's API
 * client. Every change takes its account's client before the first await,
 * so an answer that comes back after a switch is never acted on as the next
 * account (a switch starts the cache over anyway).
 */
export const channelClient = () => api().service(video.ChannelService);
export const videoClient = () => api().service(video.VideoService);
export const playbackClient = () => api().service(video.PlaybackService);
export const playlistClient = () => api().service(video.PlaylistService);
export const feedClient = () => api().service(video.VideoFeedService);

/** A refusal, in the video domain's words. */
export function alertError(error: unknown): void {
  Alert.alert(errorMessage(error, { namespace: "video" }));
}

// ───────────────────────────── Reading ─────────────────────────────

/** One of the three feeds, page by page (trending by genre). */
export function useFeed(feed: Feed, category: video.VideoCategory) {
  return useInfiniteQuery({
    queryKey: feedKey(feed, category),
    queryFn: ({ pageParam }) => loadFeedPage(feedClient, feed, category, pageParam),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  });
}

/** A video as the watch screen shows it. `videoId` is the UUID or the short ID. */
export function useVideo(videoId: string) {
  return useQuery({
    queryKey: videoKeys.video(videoId),
    queryFn: async () => (await videoClient().getVideo({ videoId })).video ?? null,
    enabled: Boolean(videoId),
  });
}

export function useRelated(videoId: string, enabled: boolean) {
  return useQuery({
    queryKey: videoKeys.related(videoId),
    queryFn: async () => (await feedClient().listRelatedVideos({ videoId, limit: 20 })).videos,
    enabled,
  });
}

export function usePlaylist(playlistId: string | undefined) {
  return useQuery({
    queryKey: videoKeys.playlist(playlistId ?? ""),
    queryFn: async () =>
      (await playlistClient().getPlaylist({ playlistId: playlistId ?? "" })).playlist ?? null,
    enabled: Boolean(playlistId),
  });
}

export function usePlaylistItems(playlistId: string | undefined) {
  return useInfiniteQuery({
    queryKey: videoKeys.playlistItems(playlistId ?? ""),
    queryFn: ({ pageParam }) =>
      playlistClient().listPlaylistItems({
        playlistId: playlistId ?? "",
        page: { limit: 50, cursor: pageParam },
      }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
    enabled: Boolean(playlistId),
  });
}

/** The playlist a video is played from (`?list=`): its videos and the one after the current. */
export function usePlaylistQueue(playlistId: string | undefined, currentId: string | undefined) {
  const playlist = usePlaylist(playlistId);
  const items = usePlaylistItems(playlistId);
  const videos =
    items.data?.pages.flatMap((page) =>
      page.items.flatMap((item) => (item.video ? [item.video] : [])),
    ) ?? [];
  return { playlist: playlist.data ?? undefined, videos, next: nextInQueue(videos, currentId) };
}

/** The viewer's playlists, built-in ones first; with `containsVideoId`, which of them hold it. */
export function useMyPlaylists(containsVideoId = "") {
  return useQuery({
    queryKey: videoKeys.myPlaylists(containsVideoId),
    queryFn: async () => (await playlistClient().listMyPlaylists({ containsVideoId })).playlists,
  });
}

/** A channel page by handle (or ID): its header and the sections of its home. */
export function useChannelPage(handle: string) {
  return useQuery({
    queryKey: videoKeys.channel(handle),
    queryFn: () => channelClient().getChannelPage({ channelId: handle }),
    enabled: Boolean(handle),
  });
}

export function useChannelVideos(
  handle: string,
  channelId: string | undefined,
  sort: video.ChannelVideoSort,
) {
  return useInfiniteQuery({
    queryKey: videoKeys.channelVideos(handle, sort, video.VideoOrigin.UNSPECIFIED),
    queryFn: ({ pageParam }) =>
      channelClient().listChannelVideos({
        channelId: channelId ?? "",
        sort,
        page: { limit: 24, cursor: pageParam },
      }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
    enabled: Boolean(channelId),
  });
}

export function useChannelPlaylists(handle: string, channelId: string | undefined) {
  return useInfiniteQuery({
    queryKey: videoKeys.channelPlaylists(handle),
    queryFn: ({ pageParam }) =>
      playlistClient().listChannelPlaylists({
        channelId: channelId ?? "",
        page: { limit: 24, cursor: pageParam },
      }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
    enabled: Boolean(channelId),
  });
}

export function useSubscriptions() {
  return useInfiniteQuery({
    queryKey: videoKeys.subscriptions,
    queryFn: ({ pageParam }) =>
      channelClient().listSubscriptions({ page: { limit: 50, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  });
}

export function useHistory() {
  return useInfiniteQuery({
    queryKey: videoKeys.history,
    queryFn: ({ pageParam }) =>
      playbackClient().listWatchHistory({ page: { limit: 30, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  });
}

// ───────────────────────────── Changing ─────────────────────────────

function patchVideo(
  queryClient: QueryClient,
  videoId: string,
  change: (item: video.Video) => video.Video,
): void {
  queryClient.setQueryData<video.Video | null>(videoKeys.video(videoId), (current) =>
    current ? change(current) : current,
  );
}

/**
 * Likes or dislikes a video (or takes it back): the thumb moves at once, the
 * server's count follows, and a refusal puts it back.
 */
export function useRate(videoId: string): (item: video.Video, next: video.Rating) => Promise<void> {
  const queryClient = useQueryClient();
  return async (item, next) => {
    const client = videoClient();
    const before = queryClient.getQueryData<video.Video | null>(videoKeys.video(videoId));
    patchVideo(queryClient, videoId, (current) => withRating(current, next));
    try {
      const response = await client.rateVideo({
        videoId: item.summary?.id ?? videoId,
        rating: next,
      });
      patchVideo(queryClient, videoId, (current) => ({
        ...current,
        viewer: current.viewer && { ...current.viewer, rating: response.rating },
        summary: current.summary && { ...current.summary, likeCount: response.likeCount },
      }));
      void queryClient.invalidateQueries({ queryKey: videoKeys.playlistItems("LL") });
    } catch (error) {
      queryClient.setQueryData(videoKeys.video(videoId), before);
      alertError(error);
    }
  };
}

/**
 * Puts a video into a playlist or takes it out (`WL` is "watch later").
 * Resolves to whether the server agreed.
 */
export function useSaveTo(): (
  playlistId: string,
  summary: Pick<video.VideoSummary, "id">,
  add: boolean,
) => Promise<boolean> {
  const queryClient = useQueryClient();
  return async (playlistId, summary, add) => {
    const client = playlistClient();
    try {
      if (add) await client.addPlaylistItem({ playlistId, videoId: summary.id });
      else await client.removePlaylistItem({ playlistId, videoId: summary.id });
      void queryClient.invalidateQueries({ queryKey: videoKeys.playlists });
      if (playlistId === "WL") void queryClient.invalidateQueries({ queryKey: videoKeys.videos });
      return true;
    } catch (error) {
      alertError(error);
      return false;
    }
  };
}

/** Starts a private playlist with the video in it. */
export function useCreatePlaylist(): (title: string, videoId: string) => Promise<boolean> {
  const queryClient = useQueryClient();
  return async (title, videoId) => {
    const client = playlistClient();
    try {
      await client.createPlaylist({ title, videoId, visibility: common.Visibility.PRIVATE });
      void queryClient.invalidateQueries({ queryKey: videoKeys.playlists });
      return true;
    } catch (error) {
      alertError(error);
      return false;
    }
  };
}

/** Subscribes (at `level`) or unsubscribes; every list that shows the channel is read again. */
export function useSubscribe(): (
  channelId: string,
  subscribe: boolean,
  options?: { level?: video.NotificationLevel; sourceVideoId?: string },
) => Promise<boolean> {
  const queryClient = useQueryClient();
  return async (channelId, subscribe, options = {}) => {
    const client = channelClient();
    try {
      if (subscribe)
        await client.subscribe({
          channelId,
          notificationLevel: options.level ?? video.NotificationLevel.UNSPECIFIED,
          sourceVideoId: options.sourceVideoId ?? "",
        });
      else await client.unsubscribe({ channelId });
      void queryClient.invalidateQueries({ queryKey: videoKeys.channels });
      void queryClient.invalidateQueries({ queryKey: videoKeys.videos });
      void queryClient.invalidateQueries({ queryKey: videoKeys.subscriptionsFeed });
      return true;
    } catch (error) {
      alertError(error);
      return false;
    }
  };
}

/** Takes a video out of the watch history (and forgets where it was left). */
export function useForget(): (videoId: string) => Promise<void> {
  const queryClient = useQueryClient();
  return async (videoId) => {
    const client = playbackClient();
    try {
      await client.removeWatchHistoryItem({ videoId });
      void queryClient.invalidateQueries({ queryKey: videoKeys.history });
      patchVideoEverywhere(queryClient, videoId, (summary) => ({
        ...summary,
        resumePositionSeconds: 0,
        watched: false,
      }));
    } catch (error) {
      alertError(error);
    }
  };
}

/** Pauses or resumes recording the history, or clears it. */
export function useHistorySettings() {
  const queryClient = useQueryClient();
  const refresh = () => void queryClient.invalidateQueries({ queryKey: videoKeys.history });
  return {
    async setPaused(paused: boolean) {
      const client = playbackClient();
      try {
        await client.setWatchHistoryPaused({ paused });
        refresh();
      } catch (error) {
        alertError(error);
      }
    },
    async clear() {
      const client = playbackClient();
      try {
        await client.clearWatchHistory({});
        refresh();
      } catch (error) {
        alertError(error);
      }
    },
  };
}
