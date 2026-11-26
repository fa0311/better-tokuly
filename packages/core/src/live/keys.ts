import type { live, video } from "@tokuly/api";

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
