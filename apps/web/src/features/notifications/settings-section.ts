import { Bell } from "lucide-react";
import type { SettingsSection } from "#/seams/settings";

/** The row of `/settings` that leads to the notification settings. */
export default {
  id: "notifications",
  order: 50,
  to: "/notifications/settings",
  icon: Bell,
  tint: "bg-surface text-ink",
  title: "settings:sections.notifications.title",
  hint: "settings:sections.notifications.hint",
} satisfies SettingsSection;
