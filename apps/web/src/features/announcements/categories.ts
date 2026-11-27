import type { admin } from "@tokuly/api";

/**
 * The categories an announcement can carry, in the order the filter shows
 * them: UPDATE, EVENT, MAINTENANCE, IMPORTANT of `admin.AnnouncementCategory`.
 * Plain numbers, so the route reading `?category=` loads none of the admin
 * API code; `categories.test.ts` holds them to the enum.
 */
export const ANNOUNCEMENT_CATEGORIES = [
  1, 3, 2, 4,
] as const satisfies readonly admin.AnnouncementCategory[];

export type AnnouncementCategory = (typeof ANNOUNCEMENT_CATEGORIES)[number];
