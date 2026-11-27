import type { admin } from "@tokuly/api";
import { REPORT_CATEGORIES, SUBJECT_TYPES } from "../filter-values";
import { parseChoice, parseEnum, parseText } from "../params";

/**
 * The queue's filters as they appear in the address bar, so a filtered queue
 * can be bookmarked and survives opening a case and coming back. Defaults
 * (open cases, newest first, everyone's) are left out of the URL.
 */
export interface QueueSearch {
  status?: "resolved";
  category?: admin.ReportCategory;
  /** `mine`, `unassigned`, or the ID of one moderator. */
  assignee?: string;
  type?: (typeof SUBJECT_TYPES)[number];
  /** Only cases about content of this account. */
  owner?: string;
  sort?: "oldest" | "reports";
}

export function parseQueueSearch(search: Record<string, unknown>): QueueSearch {
  return {
    status: parseChoice(search.status, ["resolved"]),
    category: parseEnum(search.category, REPORT_CATEGORIES),
    assignee: parseText(search.assignee),
    type: parseEnum(search.type, SUBJECT_TYPES),
    owner: parseText(search.owner),
    sort: parseChoice(search.sort, ["oldest", "reports"]),
  };
}
