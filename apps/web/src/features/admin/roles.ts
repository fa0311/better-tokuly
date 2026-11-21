import { useMe } from "#/stores/session";

/** What the signed-in account may do in the console, derived from its roles. */
export interface StaffAccess {
  /** Holds any staff role: may open the console at all. */
  staff: boolean;
  /** The reports queue and account administration. Admins hold this too. */
  moderator: boolean;
  /** Feature flags, announcements, server stats, the audit log and role changes. */
  admin: boolean;
}

export type StaffLevel = keyof StaffAccess;

export function staffAccess(roles: readonly string[] | undefined): StaffAccess {
  const has = (role: string) => roles?.includes(role) ?? false;
  const admin = has("admin");
  const moderator = admin || has("moderator");
  return { staff: moderator || has("staff"), moderator, admin };
}

/** The console access of whoever is signed in. */
export function useStaffAccess(): StaffAccess {
  return staffAccess(useMe()?.roles);
}

/** The roles an account can hold, in the order they are listed everywhere. */
export const STAFF_ROLES = ["admin", "moderator", "staff"] as const;
export type StaffRole = (typeof STAFF_ROLES)[number];
