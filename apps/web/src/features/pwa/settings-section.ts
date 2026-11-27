import { Smartphone } from "lucide-react";
import type { SettingsSection } from "#/seams/settings";

/** The row of `/settings` that leads to "use as an app". */
export default {
  id: "app",
  order: 70,
  to: "/pwa",
  icon: Smartphone,
  tint: "bg-ink text-surface",
  title: "pwa:settingsRow.title",
  hint: "pwa:settingsRow.hint",
} satisfies SettingsSection;
