import { admin } from "@tokuly/api";
import { api } from "#/lib/api";

/** The update log's read side lives in the admin domain's AnnouncementService. */
export const announcementClient = () => api().service(admin.AnnouncementService);

/** The categories an announcement can carry, in the order the filter shows them. */
export const ANNOUNCEMENT_CATEGORIES = [admin.AnnouncementCategory.UPDATE, admin.AnnouncementCategory.EVENT, admin.AnnouncementCategory.MAINTENANCE, admin.AnnouncementCategory.IMPORTANT] as const;

export type AnnouncementCategory = (typeof ANNOUNCEMENT_CATEGORIES)[number];

/** Translation key (in the `announcements` namespace) of a category's name. */
export const categoryKey = (category: admin.AnnouncementCategory) => `categories.${admin.AnnouncementCategory[category] ?? "UPDATE"}`;
