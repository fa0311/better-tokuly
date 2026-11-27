import type { admin } from "@tokuly/api";
import { ACTION_KINDS } from "../filter-values";
import { parseEnum, parseText } from "../params";

/** The moderation log's filters, kept in the address bar. */
export interface LogSearch {
  /** Actions against this account: its moderation history. */
  user?: string;
  /** Actions taken by this staff member. */
  moderator?: string;
  kind?: admin.ModerationActionKind;
  /** Actions belonging to this case. */
  case?: string;
}

export function parseLogSearch(search: Record<string, unknown>): LogSearch {
  return {
    user: parseText(search.user),
    moderator: parseText(search.moderator),
    kind: parseEnum(search.kind, ACTION_KINDS),
    case: parseText(search.case),
  };
}
