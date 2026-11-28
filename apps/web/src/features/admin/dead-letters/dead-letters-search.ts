import { parseText } from "../params";

export interface DeadLettersSearch {
  /** Only what failed in consumers whose name starts with this. */
  consumer?: string;
}

export function parseDeadLettersSearch(search: Record<string, unknown>): DeadLettersSearch {
  return { consumer: parseText(search.consumer) };
}
