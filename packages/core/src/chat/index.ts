/**
 * Chat logic every app shares: query keys, the server's limits, reading
 * conversations and messages (who, which list, what may be done), the
 * message window and its rows, text tokens and times, preview sentences.
 * Cache wiring, live state and outgoing messages stay in each app.
 */
export {
  type Capabilities,
  capabilitiesOf,
  capabilitiesQuery,
  DEFAULT_CAPABILITIES,
} from "./capabilities";
export {
  aheadOf,
  dropConversation,
  findInLists,
  type ListData,
  type ListPage,
  listAfter,
  patchConversation,
  putConversation,
  putInLists,
  readConversation,
} from "./conversation-cache";
export { chatKeys, LIST_FILTERS, type ListFilter } from "./keys";
export {
  readTimeline,
  type Stored,
  storeCreated,
  storeOwnRead,
  storeReaction,
  storeUpdated,
  writeTimeline,
} from "./message-cache";
export {
  abilitiesOf,
  canWrite,
  compareConversations,
  countsAsUnread,
  isGroup,
  isMuted,
  isUserContent,
  joinedParticipants,
  laterSeq,
  listOf,
  type MessageAbilities,
  manages,
  mentionsUser,
  participantState,
  peerOf,
  ringsFor,
} from "./model";
export { chatSentences } from "./preview";
export { chatTimes, fileSize, mentionAt, playTime, type Token, tokenize } from "./text";
export {
  buildRows,
  isBubble,
  leavesGap,
  type MessagePage,
  mergeMessage,
  type PendingState,
  type Row,
  reactionsAfter,
  rowKey,
  type Timeline,
  timelineOf,
  withCreated,
  withNewer,
  withOlder,
  withReaction,
  withUpdated,
} from "./timeline";
export { adjustUnread, NO_UNREAD, storeUnread, type Unread, unreadOf } from "./unread";
export { PEAK_COUNT, peaksOf, waveformBars } from "./waveform";
