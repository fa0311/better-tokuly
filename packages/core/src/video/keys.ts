import type { video } from "@tokuly/api";

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
