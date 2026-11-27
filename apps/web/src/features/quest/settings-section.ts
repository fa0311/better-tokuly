import { Trophy } from "lucide-react";
import type { SettingsSection } from "#/seams/settings";

/** The row of `/settings` that leads to the quest settings: rankings, profile, streak reminders. */
export default {
  id: "quest",
  order: 55,
  to: "/quest/settings",
  icon: Trophy,
  tint: "bg-sun text-[#17122b]",
  title: "quest:settingsRow.title",
  hint: "quest:settingsRow.hint",
} satisfies SettingsSection;
