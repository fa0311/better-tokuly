import { parseText } from "../params";

export interface FlagsSearch {
  q?: string;
  /** The account the "evaluate for a user" panel is showing. */
  evaluate?: string;
}

export function parseFlagsSearch(search: Record<string, unknown>): FlagsSearch {
  return { q: parseText(search.q), evaluate: parseText(search.evaluate) };
}
