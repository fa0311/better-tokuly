/**
 * What the video feature shares with other features.
 *
 * Showing a video anywhere — a search result, a friend's activity, a chat
 * message, a note — is a `VideoCard` (it leads to the watch page and carries
 * "watch later" and "save to playlist"):
 *
 *   <VideoCard video={summary} layout="row" source={video.TrafficSource.SEARCH} />
 *
 * Playing a processed upload — a short, a live archive, a clip — is the
 * `VideoPlayer`: adaptive HLS with the app's controls, keyboard and touch.
 *
 *   <VideoPlayer source={asset.video} subtitles={tracks} chapters={chapters} startAt={30} autoPlay />
 *
 * Channels: `ChannelAvatar` / `ChannelName` draw a `ChannelSummary`,
 * `SubscribeButton` subscribes to one. A feature adds a tab to every channel
 * page (`/c/<handle>/<id>`) by default-exporting a `ChannelTab` from
 * `src/features/<area>/channel-tab.tsx` — see `ChannelTab`.
 *
 * `useMyChannels()` is the signed-in person's channels (the identities they
 * can post, stream and comment as), default first.
 */
export { useMyChannels } from "./api";
export type { ChannelTab } from "./channel/tabs";
export { formatClock, formatCount } from "./format";
export { announceTrafficSource } from "./playback-session";
export { type PlayerHandle, VideoPlayer, type VideoPlayerProps } from "./player/player";
export { openSaveDialog } from "./save-dialog";
export { SubscribeButton, type SubscribeButtonProps } from "./subscribe-button";
export {
  ChannelAvatar,
  ChannelName,
  Thumbnail,
  VideoCard,
  type VideoCardProps,
  VideoCardSkeleton,
  VideoGrid,
} from "./video-card";
