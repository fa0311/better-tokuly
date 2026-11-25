/**
 * Chat logic every app shares: query keys, the server's limits, reading
 * conversations and messages (who, which list, what may be done), the
 * message window and its rows, text tokens and times, preview sentences.
 * Cache wiring, live state and outgoing messages stay in each app.
 */
export { consentStorageKey } from "./assist-consent";
export {
  type Capabilities,
  capabilitiesOf,
  capabilitiesQuery,
  DEFAULT_CAPABILITIES,
} from "./capabilities";
export { chatKeys, LIST_FILTERS, type ListFilter } from "./keys";
export * from "./model";
export { chatSentences } from "./preview";
export { chatTimes, fileSize, mentionAt, playTime, type Token, tokenize } from "./text";
export * from "./timeline";
export { PEAK_COUNT, peaksOf, waveformBars } from "./waveform";
