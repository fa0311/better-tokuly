/**
 * Search logic every app shares: the kinds and their names, the fields a
 * domain publishes with a hit, where a hit leads, query keys and options of
 * trends and recent searches, tag-only queries, and the suggestion lists
 * under a search field. The screens, the field and its debouncing stay in
 * each app.
 */
export { searchKeys } from "./keys";
export {
  extraFlag,
  extraNumber,
  extraText,
  type HitExtra,
  hitExtra,
  isKind,
  KINDS,
  type Kind,
  kindFromSlug,
  kindSlug,
} from "./kinds";
export { hitPath, isAppPath } from "./paths";
export { searchQueries } from "./queries";
export {
  fromTagQuery,
  isTagQuery,
  MAX_QUERY_LENGTH,
  SORTS,
  type Sort,
  sortValue,
  toTagQuery,
} from "./query";
export {
  idleSuggestions,
  type SearchSuggestion,
  type SuggestionGroup,
  typedSuggestions,
} from "./suggestions";
