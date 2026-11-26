/**
 * Video logic every app shares: query keys, the cache edits for one video,
 * the feeds (their pages, sources and what they show), the watch page's rules
 * (where playback starts, the chapter playing, the description's links and
 * times, who may see it, why it cannot play, the thumb's count), playlists'
 * names and order, playback reporting, clocks, counts and ages, and the genres.
 * The player, its controls and the studio stay in each app.
 */
export { patchVideoEverywhere, removeVideoEverywhere } from "./cache";
export {
  type Dismissal,
  FEED_SOURCES,
  FEEDS,
  type Feed,
  type FeedPage,
  type FeedSearch,
  feedKey,
  feedVideos,
  loadFeedPage,
  validateFeedSearch,
} from "./feeds";
export {
  formatAge,
  formatClock,
  formatCount,
  parseClock,
  parseTimeParam,
  videoIdFromInput,
} from "./format";
export { videoKeys } from "./keys";
export { CATEGORIES, categoryKey } from "./labels";
export { PlaybackReporter, type PlaybackReporterDeps } from "./playback";
export {
  audienceKey,
  type BlockKind,
  blockKind,
  chapterAt,
  type DescriptionPart,
  descriptionParts,
  imageUrl,
  nextInQueue,
  playlistTitle,
  startPosition,
  validateWatchSearch,
  type WatchSearch,
  withRating,
} from "./watch";
