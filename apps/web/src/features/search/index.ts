/**
 * What the search feature shares with other features.
 *
 * - `openQuickSearch(query?)` / `closeQuickSearch()`: open the command palette
 *   (typeahead that jumps straight to a person, a tag or a result page) from a
 *   button, optionally with text typed already. The palette itself is mounted
 *   for the whole app by `background.tsx` and opens with Cmd/Ctrl+K or "/".
 * - `searchHref(query, { kind?, sort?, owner? })`: the address of a result
 *   page, e.g. `searchHref("#散歩")` for a hashtag, or
 *   `searchHref("カフェ", { owner: user.id })` for "search this profile".
 * - `SEARCH_KINDS` / `SearchKindId`: the kinds a result page has tabs for.
 */
export { KINDS as SEARCH_KINDS, type Kind as SearchKindId } from "@tokuly/core/search";
export { searchHref } from "./hit-href";
export { closeQuickSearch, openQuickSearch } from "./palette";
