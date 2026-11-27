export const STAFF_TABS = ["indexes", "documents", "blocked", "synonyms"] as const;
export type StaffTab = (typeof STAFF_TABS)[number];
