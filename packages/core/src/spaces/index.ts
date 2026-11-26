/**
 * Spaces logic every app shares: query keys and options of rooms and
 * hangouts, the rules of the stage (who may talk, who moderates, its order),
 * what a pushed `SpacesEvent` changes in the cache, and tag input. The audio
 * connection (LiveKit), the realtime subscription and emoji bursts stay in
 * each app.
 */
export {
  applySpacesEvent,
  forgetHangout,
  patchHangout,
  patchSpace,
  rememberHangout,
  rememberSpace,
} from "./cache";
export { spacesKeys } from "./keys";
export {
  insideOf,
  isModerator,
  mergeHangout,
  mergeSpace,
  minutesOf,
  nameOf,
  onStage,
  parseTags,
  sortStage,
  withCounts,
  withoutParticipant,
  withParticipant,
  withPresence,
} from "./model";
export { type HangoutBoard, type ParticipantPage, type SpacePage, spacesQueries } from "./queries";
