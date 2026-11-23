import { Trophy } from "lucide-react";
import i18n from "#/lib/i18n";
import type { SettingsSection } from "#/seams/settings";

/** The row of `/settings` that leads to the quest settings: rankings, profile, streak reminders. */
export default {
  id: "quest",
  order: 55,
  to: "/quest/settings",
  icon: Trophy,
  tint: "bg-sun text-[#17122b]",
  title: () => i18n.t("quest:settingsRow.title"),
  hint: () => i18n.t("quest:settingsRow.hint"),
} satisfies SettingsSection;
