/**
 * Shorts logic every app shares: query keys, the limits of a short, captions
 * and their completion, which feed a link opens and what it reports as the
 * playback source, which slides around the current one load, and labels.
 * The player, gestures, studio and feeds' query wiring stay in each app.
 */
export { shortKeys } from "./keys";
export {
  type CaptionPart,
  captionParts,
  clock,
  completeToken,
  contextOfSearch,
  type FeedContext,
  type FeedKind,
  imageUrl,
  LISTS,
  type ListName,
  MAX_ASPECT,
  MAX_CAPTION,
  MAX_SECONDS,
  playbackSource,
  type ShortSearch,
  type SlideLoad,
  searchOfContext,
  shortSubject,
  slideLoad,
  soundTitle,
  tokenAtCaret,
  validateShortSearch,
} from "./model";
