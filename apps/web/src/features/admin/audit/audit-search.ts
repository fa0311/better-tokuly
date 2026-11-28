import { parseChoice, parseText } from "../params";

/** The kinds of thing an audit entry can be about. */
export const TARGET_TYPES = [
  "user",
  "case",
  "subject",
  "flag",
  "announcement",
  "dead_letters",
] as const;
export type TargetType = (typeof TARGET_TYPES)[number];

/** The audit log's filters, kept in the address bar. Times are local `YYYY-MM-DDTHH:mm`. */
export interface AuditSearch {
  actor?: string;
  /** An exact action (`user.suspended`) or a prefix ending in a dot (`user.`). */
  action?: string;
  targetType?: TargetType;
  targetId?: string;
  targetUser?: string;
  since?: string;
  until?: string;
}

export function parseAuditSearch(search: Record<string, unknown>): AuditSearch {
  return {
    actor: parseText(search.actor),
    action: parseText(search.action),
    targetType: parseChoice(search.targetType, TARGET_TYPES),
    targetId: parseText(search.targetId),
    targetUser: parseText(search.targetUser),
    since: parseText(search.since),
    until: parseText(search.until),
  };
}
