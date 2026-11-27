import { VolumeX } from "lucide-react";
import type { SettingsSection } from "#/seams/settings";

/** The row of `/settings` that leads to the notes mutes: muted words and hidden renotes. */
export default {
  id: "notes-mutes",
  order: 44,
  to: "/settings/notes",
  icon: VolumeX,
  tint: "bg-sun text-[#17122b]",
  title: "notes:mutes.settingsTitle",
  hint: "notes:mutes.settingsHint",
} satisfies SettingsSection;
