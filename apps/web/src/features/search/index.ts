/**
 * What the search feature shares with other features.
 *
 * - `<QuickSearch />`: the command palette (typeahead that jumps straight to
 *   a person, a tag or a result page). It is already mounted for the whole
 *   app by `background.tsx` and opens with Cmd/Ctrl+K or "/"; nobody needs to
 *   render it again.
 * - `openQuickSearch(query?)` / `closeQuickSearch()`: open it from a button
 *   (a search icon in a header), optionally with text typed already.
 * - `searchHref(query, { kind?, sort?, owner? })`: the address of a result
 *   page, e.g. `searchHref("#散歩")` for a hashtag, or
 *   `searchHref("カフェ", { owner: user.id })` for "search this profile".
 * - `SEARCH_KINDS` / `SearchKindId`: the kinds a result page has tabs for.
 */
export { KINDS as SEARCH_KINDS, type Kind as SearchKindId } from "./kinds";
export { searchHref } from "./params";
export { closeQuickSearch, openQuickSearch, QuickSearch } from "./quick-search";
