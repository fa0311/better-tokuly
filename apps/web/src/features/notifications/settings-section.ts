import { Bell } from "lucide-react";
import i18n from "#/lib/i18n";
import type { SettingsSection } from "#/seams/settings";

/** The row of `/settings` that leads to the notification settings. */
export default {
  id: "notifications",
  order: 50,
  to: "/notifications/settings",
  icon: Bell,
  tint: "bg-surface text-ink",
  title: () => i18n.t("settings:sections.notifications.title"),
  hint: () => i18n.t("settings:sections.notifications.hint"),
} satisfies SettingsSection;
