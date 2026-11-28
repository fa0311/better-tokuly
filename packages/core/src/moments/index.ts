/**
 * Moments (いまの瞬間) logic every app shares: query keys and options, where
 * the caller's day stands (the prompt, the two-minute window), what a locked
 * moment may show, the RealMoji faces, how late a moment was, and the cache
 * edits for what the caller does and what a pushed `MomentsEvent` changes.
 * The camera, the screens and the realtime subscription stay in each app.
 */
export {
  applyMomentsEvent,
  forgetUserMoments,
  patchMoment,
  rememberPostedMoment,
  removeMoment,
  setMyReaction,
} from "./cache";
export { momentKeys } from "./keys";
export { momentQueries } from "./queries";
export {
  CAPTION_MAX,
  cleanCaption,
  type MomentPhase,
  type MomentPicture,
  momentPhase,
  momentPicture,
  REALMOJI_FACES,
  reactionBlocker,
  realmojiGlyph,
} from "./rules";
export { countdownText, latenessText } from "./text";
