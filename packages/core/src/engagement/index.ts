/**
 * Engagement logic every app shares: what a subject is, its counts and the
 * viewer's reactions as the controls read them, how a tap changes them before
 * the server answers, realtime counters on top, and the query keys. Loading,
 * the controls and their toasts stay in each app.
 */
export { engagementKeys } from "./keys";
export {
  compactCount,
  DEFAULT_RULES,
  type EmojiCount,
  type Engagement,
  emojiReaction,
  isCustomEmoji,
  LIKE,
  type ReactionChange,
  type ReactionPicture,
  type ReactionRules,
  reactionFields,
  readReaction,
  representative,
  type Subject,
  sameReaction,
  subjectKey,
  subjectTopic,
  toEngagement,
  toPicture,
  type ViewerReaction,
  withCounters,
  withReactionChange,
  withViewerReaction,
} from "./model";
