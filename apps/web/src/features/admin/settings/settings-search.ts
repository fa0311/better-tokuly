import { parseChoice, parseText } from "../params";

export const FILTERS = ["all", "console", "locked", "pending", "rejected"] as const;
export type Filter = (typeof FILTERS)[number];

export interface SettingsSearch {
  q?: string;
  show?: Filter;
}

export function parseSettingsSearch(search: Record<string, unknown>): SettingsSearch {
  return { q: parseText(search.q), show: parseChoice(search.show, FILTERS) };
}
