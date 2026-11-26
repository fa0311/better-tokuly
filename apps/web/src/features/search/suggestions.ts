import type { SuggestionGroup as SearchGroup, SearchSuggestion } from "@tokuly/core/search";
import type { HubTile } from "#/seams/hub";

/** The groups a suggestion list is shown in: search's own, and the parts of the app. */
export type SuggestionGroup = SearchGroup | "destinations";

/** One line of the suggestion list under a search field. */
export type Suggestion =
  | SearchSuggestion
  /** A part of the app itself (a hub tile): the wallet, the settings, the question box… */
  | { id: string; group: "destinations"; type: "destination"; tile: HubTile };

/** How many parts of the app are offered for one input. */
const MAX_DESTINATIONS = 4;

/**
 * The hub tiles a text points at: by the tile's name in the current language
 * or one of its keywords, ignoring case. The quick search lists them first,
 * so it doubles as the app's launcher.
 */
export function destinationSuggestions(text: string, tiles: readonly HubTile[]): Suggestion[] {
  const query = text.trim().toLowerCase();
  if (!query) return [];
  const matches = (value: string) => value.toLowerCase().includes(query);
  return tiles
    .filter((tile) => matches(tile.label()) || (tile.keywords ?? []).some(matches))
    .slice(0, MAX_DESTINATIONS)
    .map((tile) => ({
      type: "destination",
      group: "destinations",
      id: `destination:${tile.id}`,
      tile,
    }));
}
