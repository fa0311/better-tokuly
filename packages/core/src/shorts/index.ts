/**
 * Shorts logic every app shares: query keys, the limits of a short, captions
 * and their completion, which feed a link opens and what it reports as the
 * playback source, the pages of every feed and grid, the cache edits for one
 * short, how long a stay on one lasted, which slides around the current one
 * load, and labels.
 * The player, gestures, studio and feeds' query wiring stay in each app.
 */
export { patchShortEverywhere, patchViewer, removeShortEverywhere } from "./cache";
export { shortKeys } from "./keys";
export {
  type FeedEntry,
  isSwipeOnly,
  type ListOptions,
  type ListPage,
  listKey,
  loadListPage,
  type ShortClients,
} from "./lists";
export {
  type CaptionPart,
  captionParts,
  clock,
  completeToken,
  imageUrl,
  MAX_ASPECT,
  MAX_CAPTION,
  MAX_SECONDS,
  playbackSource,
  type SlideLoad,
  shortSubject,
  slideLoad,
  soundTitle,
  tokenAtCaret,
} from "./model";
export {
  contextOfSearch,
  type FeedContext,
  type FeedKind,
  LISTS,
  type ListName,
  type ShortSearch,
  searchOfContext,
  validateShortSearch,
} from "./search";
export { type Stay, StayMeter } from "./stay";
