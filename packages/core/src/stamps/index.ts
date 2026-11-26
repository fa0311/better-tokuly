/**
 * Stamp logic every app shares: query keys and options of the store and the
 * library, cache edits after installing, favouriting or editing, and the
 * names of pack states and send contexts. Buying, the studio, review and the
 * stamp editor stay in the web app until another app has them.
 */
export { markUninstalled, patchPack, patchStamp, refreshHoldings } from "./cache";
export { stampKeys } from "./keys";
export { contextKey, statusKey } from "./model";
export { type ShelfFilter, stampQueries } from "./queries";
