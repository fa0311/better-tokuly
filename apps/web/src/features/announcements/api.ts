import { admin } from "@tokuly/api";
import { api } from "#/lib/api";

/** The update log's read side lives in the admin domain's AnnouncementService. */
export const announcementClient = () => api().service(admin.AnnouncementService);

/** Translation key (in the `announcements` namespace) of a category's name. */
export const categoryKey = (category: admin.AnnouncementCategory) =>
  `categories.${admin.AnnouncementCategory[category] ?? "UPDATE"}`;
