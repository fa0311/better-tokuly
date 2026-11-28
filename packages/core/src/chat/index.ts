/**
 * Chat logic every app shares: query keys, the server's limits, reading
 * conversations and messages (who, which list, what may be done), the
 * message window, its rows and its paging, realtime events applied to the
 * cache, the unread totals, text tokens and times, preview sentences, how
 * a message on its way is drawn and queued. Live state (typing, read
 * cursors, calls) and the uploads of outgoing messages stay in each app.
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
  putAnswer,
  putConversation,
  readConversation,
} from "./conversation-cache";
export { applyChatEvent, type ChatSync, type EventContext, resynchronize } from "./events";
export { chatKeys, LIST_FILTERS, type ListFilter } from "./keys";
export {
  readTimeline,
  type Stored,
  storeAnswer,
  storeCreated,
  storeOwnRead,
  storeReaction,
  storeUpdated,
  toggleReaction,
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
export {
  bigEmojiCount,
  type OutgoingMessage,
  type OutgoingPlace,
  placeOf,
  previewMessage,
  SendQueue,
  sendChangesViewer,
} from "./outgoing";
export { type FetchPage, MessagePager, type Position } from "./pager";
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
export { UnreadTotals } from "./totals";
export { adjustUnread, NO_UNREAD, storeUnread, type Unread, unreadOf } from "./unread";
export { PEAK_COUNT, peaksOf, waveformBars } from "./waveform";
