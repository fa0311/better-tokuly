import type { admin } from "@tokuly/api";
import { ACCOUNT_STATES } from "../filter-values";
import { parseChoice, parseEnum, parseText } from "../params";
import { STAFF_ROLES, type StaffRole } from "../roles";

/** The account search as it appears in the address bar. */
export interface UsersSearch {
  q?: string;
  state?: admin.AccountState;
  role?: StaffRole;
  verified?: true;
}

export function parseUsersSearch(search: Record<string, unknown>): UsersSearch {
  return {
    q: parseText(search.q),
    state: parseEnum(search.state, ACCOUNT_STATES),
    role: parseChoice(search.role, STAFF_ROLES),
    verified: search.verified === true || search.verified === "true" ? true : undefined,
  };
}
