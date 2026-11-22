import { Code, ShieldCheck } from "lucide-react";
import i18n from "#/lib/i18n";
import type { SettingsSection } from "#/seams/settings";

const INK = "text-[#17122b]";

/** The rows of `/settings` that lead here: the applications with access, and the developer platform. */
export default [
  {
    id: "dev-authorized",
    order: 35,
    to: "/developers/authorized",
    icon: ShieldCheck,
    tint: `bg-mint ${INK}`,
    title: () => i18n.t("dev:settingsRows.authorized.title"),
    hint: () => i18n.t("dev:settingsRows.authorized.hint"),
  },
  {
    id: "dev",
    order: 95,
    to: "/developers",
    icon: Code,
    tint: `bg-sky ${INK}`,
    title: () => i18n.t("dev:settingsRows.developers.title"),
    hint: () => i18n.t("dev:settingsRows.developers.hint"),
  },
] satisfies SettingsSection[];
