import { admin } from "@tokuly/api";
import { api } from "#/lib/api";

// Typed clients of the admin domain, one per service.
export const moderationClient = () => api().service(admin.ModerationService);
export const userAdminClient = () => api().service(admin.UserAdminService);
export const flagClient = () => api().service(admin.FeatureFlagService);
export const announcementAdminClient = () => api().service(admin.AnnouncementService);
export const systemClient = () => api().service(admin.SystemService);
