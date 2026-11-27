import { Code, ShieldCheck } from "lucide-react";
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
    title: "dev:settingsRows.authorized.title",
    hint: "dev:settingsRows.authorized.hint",
  },
  {
    id: "dev",
    order: 95,
    to: "/developers",
    icon: Code,
    tint: `bg-sky ${INK}`,
    title: "dev:settingsRows.developers.title",
    hint: "dev:settingsRows.developers.hint",
  },
] satisfies SettingsSection[];
