import { Smartphone } from "lucide-react";
import i18n from "#/lib/i18n";
import type { SettingsSection } from "#/seams/settings";

/** The row of `/settings` that leads to "use as an app". */
export default {
  id: "app",
  order: 70,
  to: "/pwa",
  icon: Smartphone,
  tint: "bg-ink text-surface",
  title: () => i18n.t("pwa:settingsRow.title"),
  hint: () => i18n.t("pwa:settingsRow.hint"),
} satisfies SettingsSection;
