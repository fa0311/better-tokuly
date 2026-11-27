export const STAFF_TABS = ["payouts", "users", "adjustments", "packages"] as const;
export type StaffTab = (typeof STAFF_TABS)[number];

/** Where the staff page is, as kept in the address (so a view can be linked and survives a reload). */
export interface StaffWalletSearch {
  tab?: StaffTab;
  /** The account open in the "users" tab. */
  user?: string;
  /** The payout request open in the "payouts" tab. */
  payout?: string;
}
