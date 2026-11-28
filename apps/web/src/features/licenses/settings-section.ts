import { Scale } from "lucide-react";
import type { SettingsSection } from "#/seams/settings";

/** The row of `/settings` that leads to the licences and credits. */
export default {
  id: "licenses",
  order: 99,
  to: "/settings/licenses",
  icon: Scale,
  tint: "bg-surface-2 text-ink",
  title: "licenses:settingsRow.title",
  hint: "licenses:settingsRow.hint",
} satisfies SettingsSection;
