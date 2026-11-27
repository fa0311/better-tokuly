import { parseChoice } from "../params";

const STATUS_FILTERS = ["draft", "scheduled", "published"] as const;
export type StatusFilter = (typeof STATUS_FILTERS)[number];

export interface ManagedAnnouncementsSearch {
  status?: StatusFilter;
}

export function parseManagedAnnouncementsSearch(
  search: Record<string, unknown>,
): ManagedAnnouncementsSearch {
  return { status: parseChoice(search.status, STATUS_FILTERS) };
}
