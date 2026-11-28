/**
 * Notes logic every app shares: query keys (notes, lists and antennas,
 * clips), the page of every timeline and how new rows go on top, the cache
 * edits for one note wherever it is cached and for a note just posted or
 * deleted, realtime events, how a note's text is read (length, hashtags,
 * mentions, completion), audiences and the reply rule, polls.
 * Rendering (the server's rich text), the composer, the batching of single
 * notes and of views, and what is kept on a device stay in each app.
 */
export {
  AUDIENCE_OPTIONS,
  type Audience,
  type AudienceKey,
  type AudienceOption,
  audienceFields,
  audienceOption,
  noteAudience,
  replyAudience,
  sameAudience,
  visibilityOption,
} from "./audience";
export {
  addPostedNote,
  bumpCounter,
  type CountsBefore,
  countsBefore,
  findCachedNote,
  forgetUserNotes,
  mapNotes,
  type NoteCounter,
  patchNote,
  prependNote,
  settleDeletedNote,
  type ThreadData,
  withCounters,
  withToggle,
  withViewer,
} from "./cache";
export { clipKeys, listKeys, notesKeys, type TimelineSource } from "./keys";
export {
  applyNotesEvent,
  isNoteNotification,
  type NotesEventEffect,
  putRefreshedNote,
} from "./live";
export {
  canVote,
  newPoll,
  POLL_CHOICE_MAX_CHARS,
  POLL_DURATIONS,
  POLL_MAX_CHOICES,
  type PollDraft,
  pollInput,
  pollReady,
  pollRemaining,
  pollShares,
} from "./poll";
export {
  applyCompletion,
  type Completion,
  charCount,
  completionAt,
  NOTE_MAX_CHARS,
  normalizeHashtag,
  parseNoteText,
  type Segment,
} from "./text";
export {
  countNewEntries,
  entryKey,
  loadTimelinePage,
  lookForNewEntries,
  PAGE_SIZE,
  type TimelineClients,
  type TimelineData,
  type TimelinePage,
  timelineQuery,
  withFreshEntries,
} from "./timeline";
